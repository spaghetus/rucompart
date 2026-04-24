use dashmap::DashMap;
use rayon::prelude::*;
use rhai::{Dynamic, Engine, Func};
use rucompart::{
	Compartment,
	chan::{Channel, EstablishedChannel, Many},
	compartmentalize,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
	net::SocketAddr,
	sync::{Arc, OnceLock},
	time::Duration,
};
use tokio::{
	sync::{
		Mutex,
		mpsc::{Receiver, Sender},
	},
	task::JoinHandle,
};
use tracing::{instrument, trace};

#[derive(Serialize, Deserialize, Debug)]
pub enum UpwardsMessage {
	FilterMapReduceResult(Vec<Value>),
}

#[derive(Serialize, Deserialize, Debug)]
pub enum DownwardsMessage {
	StartFilterMapReduce(String),
}

#[instrument]
pub(crate) fn guest(addr: SocketAddr) {
	tracing::debug!("Initializing...");
	let shard = ShardService {
		inner: Arc::new(Mutex::new(ShardInner {
			thread: OnceLock::new(),
			upward: OnceLock::new(),
			downward: vec![],
		})),
		store: Arc::new(DashMap::new()),
		commands: Arc::new(OnceLock::new()),
		responses: Arc::new(Mutex::new(tokio::sync::mpsc::channel(1).1)),
	};
	let server = shard.clone();
	tracing::debug!("Listening...");
	shard.listen_on_tcp(server.serve(), addr)
}

#[tarpc::service]
pub(crate) trait Shard {
	async fn start();
	async fn set_upstream(upstream: Channel<(), Many, UpwardsMessage, DownwardsMessage>);
	async fn get_downstream() -> Channel<(), Many, UpwardsMessage, DownwardsMessage>;
	async fn list() -> Vec<String>;
	async fn store(key: String, value: Value) -> Option<Value>;
	async fn load(key: String) -> Option<Value>;
	/// Takes Rhai source code.
	/// Only makes sense on shard 0.
	async fn filter_map_reduce(program: String) -> Value;
}

#[derive(Clone)]
pub(crate) struct ShardService {
	pub(crate) inner: Arc<Mutex<ShardInner>>,
	pub(crate) store: Arc<DashMap<String, Value>>,
	pub(crate) commands: Arc<OnceLock<Sender<DownwardsMessage>>>,
	pub(crate) responses: Arc<Mutex<Receiver<UpwardsMessage>>>,
}

pub(crate) struct ShardInner {
	pub(crate) thread: OnceLock<JoinHandle<()>>,
	pub(crate) upward: OnceLock<EstablishedChannel<Many, UpwardsMessage, DownwardsMessage>>,
	pub(crate) downward: Vec<EstablishedChannel<Many, DownwardsMessage, UpwardsMessage>>,
}

#[instrument(skip(inner, store, msg))]
async fn downwards_message(
	inner: &mut ShardInner,
	msg: DownwardsMessage,
	store: &DashMap<String, Value>,
) -> Option<UpwardsMessage> {
	tracing::debug!("Got horizontal message!");
	match msg {
		DownwardsMessage::StartFilterMapReduce(source) => {
			tracing::debug!("It's a map-reduce operation. Propagate down...");
			for downward in &mut inner.downward {
				let _ = downward
					.send(DownwardsMessage::StartFilterMapReduce(source.clone()))
					.await;
			}
			tracing::debug!("Preparing script...");
			let engine = Engine::new();
			let ast = engine.compile(source).unwrap();
			let filter = Func::<(String, Dynamic), bool>::create_from_ast(
				Engine::new(),
				ast.clone(),
				"filter",
			);
			let map = Func::<(String, Dynamic), Dynamic>::create_from_ast(
				Engine::new(),
				ast.clone(),
				"map",
			);
			let reduce = Func::<(Dynamic, Dynamic), Dynamic>::create_from_ast(
				Engine::new(),
				ast.clone(),
				"reduce",
			);
			let reduce = |l: Dynamic, r: Dynamic| -> Dynamic {
				match (l.is_unit(), r.is_unit()) {
					(true, true) => Dynamic::UNIT,
					(true, false) => r,
					(false, true) => l,
					(false, false) => reduce(l, r).unwrap_or_default(),
				}
			};
			tracing::debug!("Working on local data...");
			let result = store
				.par_iter()
				.filter_map(|kv| {
					filter(
						kv.key().clone(),
						rhai::serde::to_dynamic(kv.value()).unwrap(),
					)
					.unwrap_or_default()
					.then_some(kv)
				})
				.map(|kv| {
					map(
						kv.key().clone(),
						rhai::serde::to_dynamic(kv.value()).unwrap(),
					)
					.unwrap_or_default()
				})
				.reduce_with(&reduce);
			tracing::debug!("Collecting incoming data...");
			let mut received = vec![];
			for downward in &mut inner.downward {
				let Some(UpwardsMessage::FilterMapReduceResult(msg)) = downward.recv().await else {
					continue;
				};
				received.extend(msg);
			}
			tracing::debug!("Combining...");
			let result = result
				.into_par_iter()
				.map(|v| rhai::serde::from_dynamic::<Value>(&v).unwrap())
				.chain(received)
				.map(|v| rhai::serde::to_dynamic(&v).unwrap())
				.reduce_with(reduce)
				.unwrap_or_default();
			tracing::debug!(
				"Done! Passing up the chain. Here's the data so far:\n{:#?}",
				result
			);
			Some(UpwardsMessage::FilterMapReduceResult(vec![
				rhai::serde::from_dynamic(&result).unwrap(),
			]))
		}
	}
}

#[instrument(skip(inner, store, server_commands, server_responses))]
pub(crate) async fn shard_daemon(
	inner: Arc<Mutex<ShardInner>>,
	store: Arc<DashMap<String, Value>>,
	mut server_commands: Receiver<DownwardsMessage>,
	server_responses: Sender<UpwardsMessage>,
) {
	loop {
		let mut inner = inner.lock().await;
		tokio::select! {
			() = tokio::time::sleep(Duration::from_millis(10)) => {}
			msg = async {
				if let Some(upward) = inner.upward.get_mut() {
					upward.recv().await
				} else {
					tokio::time::sleep(Duration::from_secs(10_000_000)).await; unreachable!()
				}} => if let Some(msg) = msg {
				let response = tokio::select! {
					msg = downwards_message(&mut inner, msg, &store) => msg,
					() = tokio::time::sleep(Duration::from_secs(1)) => None
				};
				if let Some(response) = response {inner.upward.get_mut().unwrap().send(response).await.unwrap();}
			},
			msg = server_commands.recv() => if let Some(msg) = msg {
				let response = tokio::select! {
					msg = downwards_message(&mut inner, msg, &store) => msg,
					() = tokio::time::sleep(Duration::from_secs(1)) => None
				};
				if let Some(response) = response {server_responses.send(response).await.unwrap();}
			}
		}
	}
}

impl Shard for ShardService {
	async fn start(self, _context: ::tarpc::context::Context) {
		let (command_sender, command_receiver) = tokio::sync::mpsc::channel(1);
		let (response_sender, response_receiver) = tokio::sync::mpsc::channel(1);
		self.commands
			.set(command_sender)
			.expect("Must not run start twice");
		*self.responses.lock().await = response_receiver;
		let inner = self.inner.lock().await;
		inner
			.thread
			.set(tokio::task::spawn(shard_daemon(
				self.inner.clone(),
				self.store.clone(),
				command_receiver,
				response_sender,
			)))
			.unwrap();
	}

	async fn set_upstream(
		self,
		_context: ::tarpc::context::Context,
		upstream: Channel<(), Many, UpwardsMessage, DownwardsMessage>,
	) -> () {
		let stream = upstream.connect().await.unwrap();
		let _ = self.inner.lock().await.upward.set(stream);
	}

	async fn get_downstream(
		self,
		_context: ::tarpc::context::Context,
	) -> Channel<(), Many, UpwardsMessage, DownwardsMessage> {
		let (connection, spec) = rucompart::chan::channel().await.unwrap();
		let inner_lock = self.inner.clone();
		tokio::task::spawn(async move {
			let connection = connection.await;
			inner_lock.lock().await.downward.push(connection);
		});
		spec
	}

	async fn store(
		self,
		_context: ::tarpc::context::Context,
		key: String,
		value: Value,
	) -> Option<Value> {
		self.store.insert(key, value)
	}

	async fn load(self, _context: ::tarpc::context::Context, key: String) -> Option<Value> {
		self.store.get(&key).map(|v| v.clone())
	}

	/// Takes Rhai source code.
	/// Only makes sense on shard 0.
	async fn filter_map_reduce(
		self,
		_context: ::tarpc::context::Context,
		program: String,
	) -> Value {
		self.commands
			.get()
			.unwrap()
			.send(DownwardsMessage::StartFilterMapReduce(program))
			.await
			.unwrap();
		let Some(UpwardsMessage::FilterMapReduceResult(result)) =
			self.responses.lock().await.recv().await
		else {
			panic!("Protocol error!");
		};
		result.first().cloned().unwrap_or_default()
	}

	async fn list(self, _context: ::tarpc::context::Context) -> Vec<String> {
		self.store.iter().map(|kv| kv.key().clone()).collect()
	}
}

compartmentalize!(
	"SHARD",
	ServeShard,
	ShardService,
	ShardClient,
	async fn setup(
		service: &mut Self::Server,
		mode: rucompart::CompartmentMode,
	) -> Result<_, std::io::Error> {
		Ok(())
	}
);
