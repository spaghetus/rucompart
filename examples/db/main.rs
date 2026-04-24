#![warn(clippy::pedantic)]
use std::{
	hash::{DefaultHasher, Hasher},
	net::SocketAddr,
	ops::Rem,
	time::{Duration, Instant},
};

use crate::shard::{ShardClient, ShardService};
use clap::{Parser, Subcommand};
use rhai::Engine;
use rocket::{State, get, http::Status, post, routes, serde::json::Json};
use rucompart::compartment_connect;
use serde_json::Value;
use tarpc::context::Context;
use tokio::{runtime::Runtime, task::JoinSet};
use tracing::{Instrument, Span, debug, field::debug, instrument, level_filters::LevelFilter};

mod shard;

#[derive(Parser)]
struct Args {
	#[arg(short, default_value = "warn")]
	verbosity: LevelFilter,
	#[command(subcommand)]
	mode: DbMode,
}

#[derive(Clone, Subcommand)]
enum DbMode {
	Host {
		#[arg(short, long)]
		shards: Vec<SocketAddr>,
	},
	Guest {
		listen: SocketAddr,
	},
}

fn main() {
	let Args { mode, verbosity } = Args::parse();
	tracing_subscriber::fmt().with_max_level(verbosity).init();

	match mode {
		DbMode::Host { shards } => Runtime::new().unwrap().block_on(host(shards)),
		DbMode::Guest { listen } => shard::guest(listen),
	}
}

#[instrument(skip(shards))]
async fn host(shards: Vec<SocketAddr>) {
	let shards = shards
		.into_iter()
		.map(|addr| {
			compartment_connect!(
				Some(addr.to_string()),
				ShardService,
				|| -> <ShardService as rucompart::Compartment>::Server { unreachable!() },
				ShardClient
			)
		})
		.collect::<JoinSet<_>>()
		.join_all()
		.instrument(Span::current())
		.await;
	#[instrument(skip(shards))]
	async fn assign_upstream(shards: &[ShardClient]) {
		let mut ctx = Context::current();
		dbg!(ctx.deadline - Instant::now());
		ctx.deadline = Instant::now() + Duration::from_mins(100);
		let [left, rest @ ..] = shards else { return };
		if rest.is_empty() {
			return;
		}
		if rest.len() == 1 {
			let upstream = left
				.get_downstream(ctx)
				.instrument(Span::current())
				.await
				.unwrap();
			rest[0]
				.set_upstream(ctx, upstream)
				.instrument(Span::current())
				.await
				.unwrap();
		}
		let (left_shards, right_shards) = rest.split_at(rest.len() / 2);
		if let Some(left_leaf) = left_shards.first() {
			left_leaf
				.set_upstream(ctx, left.get_downstream(ctx).await.unwrap())
				.instrument(Span::current())
				.await
				.unwrap();
		}
		if let Some(right_leaf) = right_shards.first() {
			right_leaf
				.set_upstream(ctx, left.get_downstream(ctx).await.unwrap())
				.instrument(Span::current())
				.await
				.unwrap();
		}
		Box::pin(assign_upstream(left_shards))
			.instrument(Span::current())
			.await;
		Box::pin(assign_upstream(right_shards))
			.instrument(Span::current())
			.await;
	}
	assign_upstream(&shards).await;
	for shard in &shards {
		shard
			.start(Context::current())
			.instrument(Span::current())
			.await
			.unwrap();
	}

	rocket::build()
		.manage(shards)
		.mount("/", routes![get_value, put_value, map_reduce, enumerate])
		.launch()
		.await
		.unwrap();
}

#[get("/")]
#[instrument(skip(shards))]
async fn enumerate(shards: &State<Vec<ShardClient>>) -> Json<Vec<String>> {
	let queries: JoinSet<_> = shards
		.iter()
		.map(|shard| {
			tokio::spawn({
				let shard = shard.clone();
				async move { shard.list(Context::current()).await }
			})
		})
		.collect();
	let list = queries
		.join_all()
		.await
		.into_iter()
		.flatten()
		.filter_map(|r| match r {
			Ok(v) => Some(v),
			Err(e) => {
				eprintln!("One of our list queries failed with {e:#?}");
				None
			}
		})
		.flatten()
		.collect();
	Json(list)
}

#[get("/<key>")]
#[instrument(skip(shards))]
async fn get_value(key: String, shards: &State<Vec<ShardClient>>) -> (Status, Json<Option<Value>>) {
	let mut hasher = DefaultHasher::new();
	hasher.write(key.as_bytes());
	let shard = usize::try_from(hasher.finish()).unwrap().rem(shards.len());
	debug!("For key {key:?}, picked shard #{shard}.");
	let shard = &shards[shard];
	let v = Json(shard.load(Context::current(), key).await.unwrap());

	(
		if v.is_some() {
			Status::Ok
		} else {
			Status::NoContent
		},
		v,
	)
}

#[post("/<key>", data = "<data>")]
#[instrument(skip(shards))]
async fn put_value(
	key: &str,
	shards: &State<Vec<ShardClient>>,
	data: Json<Value>,
) -> (Status, Json<Option<Value>>) {
	let mut hasher = DefaultHasher::new();
	hasher.write(key.as_bytes());
	let shard = usize::try_from(hasher.finish()).unwrap().rem(shards.len());
	debug!("For key {key:?}, picked shard #{shard}.");
	let shard = &shards[shard];

	let v = Json(
		match shard
			.store(Context::current(), key.to_string(), data.0)
			.await
		{
			Ok(v) => v,
			Err(e) => {
				eprintln!("Rpc failed with {e:#?}");
				None
			}
		},
	);

	(
		if v.is_some() {
			Status::Ok
		} else {
			Status::NoContent
		},
		v,
	)
}

#[post("/", data = "<source>")]
#[instrument(skip(shards, source))]
async fn map_reduce(
	shards: &State<Vec<ShardClient>>,
	source: String,
) -> Result<Json<Value>, String> {
	let shard = &shards[0];
	let engine = Engine::new();
	engine.compile(&source).map_err(|e| e.to_string())?;
	Ok(Json(
		shard
			.filter_map_reduce(Context::current(), source)
			.await
			.unwrap(),
	))
}
