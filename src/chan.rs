use core::marker::PhantomData;
use futures::{StreamExt, prelude::*};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::net::SocketAddr;
use tarpc::{
	serde_transport::Transport, tokio_serde::formats::Json, tokio_util::codec::LengthDelimitedCodec,
};
use thiserror::Error;
use tokio::{
	io::AsyncWriteExt,
	net::{TcpListener, TcpStream},
	task::JoinSet,
};

#[derive(Serialize, Deserialize, Debug)]
pub struct Channel<C, K, S, R> {
	pub addresses: Vec<SocketAddr>,
	pub inner: C,
	_state: PhantomData<(K, S, R)>,
}

#[derive(Error, Debug)]
pub enum ChannelError {
	#[error("Something went wrong when transferring data.")]
	Io(#[from] std::io::Error),
	#[error("None of the addresses we were given picked up.")]
	CouldntConnect,
}

#[derive(Debug)]
pub struct Once;
#[derive(Debug)]
pub struct Many;

pub trait ChannelKind {}

impl ChannelKind for Once {}
impl ChannelKind for Many {}

pub type EstablishedChannel<K, S, R> = Channel<Transport<TcpStream, R, S, Json<R, S>>, K, S, R>;
pub type PlannedChannel<K, S, R> = Channel<(), K, S, R>;

#[allow(private_bounds)]
impl<K: ChannelKind, S, R> Channel<(), K, S, R> {
	pub fn new(addresses: Vec<SocketAddr>) -> Self {
		Self {
			addresses,
			inner: (),
			_state: PhantomData,
		}
	}
}

impl<
	K: ChannelKind,
	S: Serialize + Send + Sync + Unpin + 'static,
	R: for<'de> Deserialize<'de> + Send + Sync + Unpin + 'static,
> Channel<(), K, S, R>
{
	pub async fn connect(
		self,
	) -> Result<Channel<Transport<TcpStream, R, S, Json<R, S>>, K, S, R>, ChannelError> {
		// let stream = TcpStream::connect(self.addresses.as_slice()).await?;
		let mut streams = self
			.addresses
			.clone()
			.into_iter()
			.map(|addr| {
				tokio::spawn(async move {
					match TcpStream::connect(addr).await {
						Ok(v) => Some(v),
						Err(e) => {
							eprintln!("Connecting to {addr} failed with: {e:#?}");
							None
						}
					}
				})
			})
			.collect::<JoinSet<_>>();
		let stream = loop {
			match streams.join_next().await {
				Some(Ok(Ok(Some(v)))) => break v,
				Some(_) => continue,
				None => return Err(ChannelError::CouldntConnect),
			}
		};
		let codec_builder = LengthDelimitedCodec::builder();
		let framed = codec_builder.new_framed(stream);
		let transport = tarpc::serde_transport::new(framed, Json::default());

		Ok(Channel {
			addresses: self.addresses,
			inner: transport,
			_state: PhantomData,
		})
	}
}

#[allow(private_bounds)]
impl<
	K: ChannelKind,
	S: Serialize + Send + Sync + Unpin + 'static,
	R: for<'de> Deserialize<'de> + Send + Sync + Unpin + 'static,
> From<Transport<TcpStream, R, S, Json<R, S>>> for EstablishedChannel<K, S, R>
{
	fn from(transport: Transport<TcpStream, R, S, Json<R, S>>) -> Self {
		Self {
			addresses: vec![],
			inner: transport,
			_state: PhantomData,
		}
	}
}

impl<
	S: Serialize + Send + Sync + Unpin + 'static,
	R: for<'de> Deserialize<'de> + Send + Sync + Unpin + 'static,
> Channel<Transport<TcpStream, R, S, Json<R, S>>, Once, S, R>
{
	pub async fn send(mut self, item: S) -> Result<(), std::io::Error> {
		self.inner.send(item).await?;
		Ok(())
	}

	pub async fn recv(mut self) -> Option<R> {
		self.inner.next().await.and_then(|v| v.ok())
	}
}

impl<
	S: Serialize + Send + Sync + Unpin + 'static,
	R: for<'de> Deserialize<'de> + Send + Sync + Unpin + 'static,
> Channel<Transport<TcpStream, R, S, Json<R, S>>, Many, S, R>
{
	pub async fn send(&mut self, item: S) -> Result<(), std::io::Error> {
		self.inner.send(item).await?;
		Ok(())
	}

	pub async fn recv(&mut self) -> Option<R> {
		self.inner.next().await.and_then(|v| v.ok())
	}
}

pub async fn channel<
	K: ChannelKind,
	R: Serialize + DeserializeOwned + Send + Sync + Unpin + 'static,
	S: Serialize + DeserializeOwned + Send + Sync + Unpin + 'static,
>() -> Result<
	(
		impl Future<Output = EstablishedChannel<K, S, R>>,
		Channel<(), K, R, S>,
	),
	ChannelError,
> {
	let listeners: Vec<TcpListener> = futures::stream::iter(netdev::get_interfaces())
		.then(|iface| async move { futures::stream::iter(iface.ip_addrs()) })
		.flatten()
		.then(|addr| async move { (addr, TcpListener::bind(SocketAddr::new(addr, 0)).await) })
		.filter_map(|(addr, conn)| async move {
			match conn {
				Ok(v) => Some(v),
				Err(e) => {
					eprintln!("Listening on {addr} failed with {e:#?}");
					None
				}
			}
		})
		.collect()
		.await;
	let channel_spec = Channel {
		_state: PhantomData,
		addresses: listeners.iter().map(|l| l.local_addr().unwrap()).collect(),
		inner: (),
	};
	let wait_for_incoming = async move {
		let mut connection = listeners
			.into_iter()
			.map(|l| async move { l.accept().await })
			.collect::<JoinSet<_>>();
		while let Some(Ok(result)) = connection.join_next().await {
			match result {
				Ok((stream, _socket)) => {
					let codec_builder = LengthDelimitedCodec::builder();
					let framed = codec_builder.new_framed(stream);
					let transport = tarpc::serde_transport::new(framed, Json::default());
					return EstablishedChannel::from(transport);
				}
				Err(e) => {
					eprintln!("Listening error {e}");
				}
			}
		}
		unreachable!()
	};
	Ok((wait_for_incoming, channel_spec))
}
