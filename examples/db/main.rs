use std::{
	hash::{DefaultHasher, Hasher},
	net::SocketAddr,
	ops::Rem,
};

use crate::shard::{ShardClient, ShardService};
use clap::{Parser, Subcommand};
use rhai::Engine;
use rocket::{State, get, http::Status, post, routes, serde::json::Json};
use rucompart::compartment_connect;
use serde_json::Value;
use tarpc::context::Context;
use tokio::{runtime::Runtime, task::JoinSet};

mod shard;

#[derive(Parser)]
struct Args {
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
	let Args { mode } = Args::parse();

	match mode {
		DbMode::Host { shards } => Runtime::new().unwrap().block_on(host(shards)),
		DbMode::Guest { listen } => shard::guest(listen),
	}
}

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
		.await;
	async fn assign_upstream(shards: &[ShardClient]) {
		let [left, rest @ ..] = shards else { return };
		if rest.is_empty() {
			return;
		}
		if rest.len() == 1 {
			let upstream = left.get_downstream(Context::current()).await.unwrap();
			rest[0]
				.set_upstream(Context::current(), upstream)
				.await
				.unwrap();
		}
		let (left_shards, right_shards) = rest.split_at(rest.len() / 2);
		if let Some(left_leaf) = left_shards.first() {
			left_leaf
				.set_upstream(
					Context::current(),
					left.get_downstream(Context::current()).await.unwrap(),
				)
				.await
				.unwrap();
		}
		if let Some(right_leaf) = right_shards.first() {
			right_leaf
				.set_upstream(
					Context::current(),
					left.get_downstream(Context::current()).await.unwrap(),
				)
				.await
				.unwrap();
		}
		Box::pin(assign_upstream(left_shards)).await;
		Box::pin(assign_upstream(right_shards)).await;
	}
	assign_upstream(&shards).await;
	for shard in &shards {
		shard.start(Context::current()).await.unwrap()
	}

	rocket::build()
		.manage(shards)
		.mount("/", routes![get_value, put_value, map_reduce])
		.launch()
		.await
		.unwrap();
}

#[get("/<key>")]
async fn get_value(key: String, shards: &State<Vec<ShardClient>>) -> (Status, Option<Json<Value>>) {
	let mut hasher = DefaultHasher::new();
	hasher.write(key.as_bytes());
	let shard = &shards[(hasher.finish() as usize).rem(shards.len())];
	let v = shard.load(Context::current(), key).await.unwrap().map(Json);

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
async fn put_value(
	key: String,
	shards: &State<Vec<ShardClient>>,
	data: Json<Value>,
) -> (Status, Option<Json<Value>>) {
	let mut hasher = DefaultHasher::new();
	hasher.write(key.as_bytes());
	let shard = &shards[(hasher.finish() as usize).rem(shards.len())];

	let v = shard
		.store(Context::current(), key, data.0)
		.await
		.unwrap()
		.map(Json);

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
