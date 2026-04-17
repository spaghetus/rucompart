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
pub struct ChannelSpec<C, K, S, R> {
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

#[allow(private_bounds)]
impl<K: ChannelKind, S, R> ChannelSpec<(), K, S, R> {
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
> ChannelSpec<(), K, S, R>
{
	pub async fn connect(
		self,
	) -> Result<ChannelSpec<Transport<TcpStream, R, S, Json<R, S>>, K, S, R>, ChannelError> {
		let stream = TcpStream::connect(self.addresses.as_slice()).await?;
		//  Box::pin(
		// 	futures::stream::iter(self.addresses.iter())
		// 		.then()
		// 		.filter_map(|v| async move { v.ok() }),
		// )
		// .next()
		// .await
		// .ok_or(ChannelError::CouldntConnect)?;
		let codec_builder = LengthDelimitedCodec::builder();
		let framed = codec_builder.new_framed(stream);
		let transport = tarpc::serde_transport::new(framed, Json::default());

		Ok(ChannelSpec {
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
> From<Transport<TcpStream, R, S, Json<R, S>>>
	for ChannelSpec<Transport<TcpStream, R, S, Json<R, S>>, K, S, R>
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
> ChannelSpec<Transport<TcpStream, R, S, Json<R, S>>, Once, S, R>
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
> ChannelSpec<Transport<TcpStream, R, S, Json<R, S>>, Many, S, R>
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
	R: Serialize + DeserializeOwned,
	S: Serialize + DeserializeOwned,
	K: ChannelKind,
>() -> Result<
	(
		impl Future<Output = Transport<TcpStream, R, S, Json<R, S>>>,
		ChannelSpec<(), K, R, S>,
	),
	ChannelError,
> {
	let listeners: Vec<TcpListener> = futures::stream::iter(netdev::get_interfaces())
		.then(|iface| async move { futures::stream::iter(iface.global_ip_addrs()) })
		.flatten()
		.then(|addr| TcpListener::bind(SocketAddr::new(addr, 0)))
		.filter_map(|conn| async move { conn.ok() })
		.collect()
		.await;
	let channel_spec = ChannelSpec {
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
					return transport;
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
