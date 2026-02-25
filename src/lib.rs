use std::{error::Error, marker::PhantomData, pin::Pin, task::Poll};

use fork::fork;
use futures::{Stream, StreamExt, task::SpawnError};
use serde::{Deserialize, Serialize};
use tarpc::{
	Transport,
	serde_transport::{self, Transport as STransport},
	server::{BaseChannel, Channel, Config, Serve},
	tokio_serde::formats::{Bincode, SymmetricalBincode},
	tokio_util::codec::{self, Framed, LengthDelimitedCodec},
};
use tokio::{
	io::{AsyncRead, AsyncWrite},
	net::{
		UnixDatagram, UnixStream,
		unix::pipe::{Receiver, Sender, pipe},
	},
	runtime::{Handle, Runtime},
	task::JoinHandle,
};

#[macro_export]
macro_rules! compartmentalize {
	($($serve:ident)::+, $service:ty, $client:ty, async fn setup(&self) -> Result<$_:ty, $setup_err:ty> $setup:tt) => {
		impl rucompart::Compartment for $service {
			type Error = $setup_err;

			type Server = $($serve)::+<$service>;

			type Client = $client;

			fn setup(
				&self,
			) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + Sync {
				async { $setup }
			}

			fn serve(
				server: Self::Server,
				channel: tarpc::server::BaseChannel<
					<Self::Server as tarpc::server::Serve>::Req,
					<Self::Server as tarpc::server::Serve>::Resp,
					tarpc::serde_transport::Transport<
						tokio::net::UnixStream,
						tarpc::ClientMessage<<Self::Server as tarpc::server::Serve>::Req>,
						tarpc::Response<<Self::Server as tarpc::server::Serve>::Resp>,
						tarpc::tokio_serde::formats::Bincode<
							tarpc::ClientMessage<<Self::Server as tarpc::server::Serve>::Req>,
							tarpc::Response<<Self::Server as tarpc::server::Serve>::Resp>,
						>,
					>,
				>,
			) -> impl Future<Output = ()> + Send + Sync {
				use futures::StreamExt;
				async move {
					tarpc::server::Channel::execute(channel, server.clone())
						.for_each(|response| async {
							response.await;
						})
						.await;
				}
			}
		}
	};
}

pub enum Side {
	Host,
	Compartment,
}

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
	#[error("Problem communicating")]
	IoError(#[from] tokio::io::Error),
}

pub trait Compartment: Sized + Send + Sync {
	type Error: Error + From<std::io::Error>;
	fn setup(&self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + Sync;

	fn serve(
		server: Self::Server,
		channel: BaseChannel<
			<<Self as Compartment>::Server as Serve>::Req,
			<<Self as Compartment>::Server as Serve>::Resp,
			STransport<
				UnixStream,
				tarpc::ClientMessage<<<Self as Compartment>::Server as Serve>::Req>,
				tarpc::Response<<<Self as Compartment>::Server as Serve>::Resp>,
				Bincode<
					tarpc::ClientMessage<<<Self as Compartment>::Server as Serve>::Req>,
					tarpc::Response<<<Self as Compartment>::Server as Serve>::Resp>,
				>,
			>,
		>,
	) -> impl Future<Output = ()> + Send + Sync;

	type Server: Serve + Clone + Send + Sync;
	type Client: Send + Sync + 'static;

	fn spawn(
		self,
		server: impl FnOnce() -> Self::Server + Send + Sync,
		client: impl FnOnce(
			STransport<
				UnixStream,
				tarpc::Response<<Self::Server as Serve>::Resp>,
				tarpc::ClientMessage<<Self::Server as Serve>::Req>,
				Bincode<
					tarpc::Response<<Self::Server as Serve>::Resp>,
					tarpc::ClientMessage<<Self::Server as Serve>::Req>,
				>,
			>,
		) -> JoinHandle<Self::Client>,
	) -> Result<(impl Future<Output = Self::Client>, i32), Self::Error>
	where
		<Self::Server as Serve>::Req: Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static,
		<Self::Server as Serve>::Resp:
			Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static,
	{
		assert!(
			Handle::try_current().is_err(),
			"This will cause untold chaos if called in a tokio runtime!"
		);
		let (host, guest) = std::os::unix::net::UnixStream::pair()?;
		host.set_nonblocking(true).unwrap();
		guest.set_nonblocking(true).unwrap();
		let fork = unsafe { libc::fork() };
		match fork {
			0 => Runtime::new().unwrap().block_on(async move {
				let guest = UnixStream::from_std(guest).unwrap();
				std::mem::forget(host);
				let codec_builder = LengthDelimitedCodec::builder();
				let framed = codec_builder.new_framed(guest);
				let transport = tarpc::serde_transport::new(framed, Bincode::default());
				let channel = BaseChannel::with_defaults(transport);
				self.setup().await.unwrap();
				Self::serve(server(), channel).await;
				std::process::exit(0)
			}),
			pid => Ok((
				async move {
					std::mem::forget(guest);
					let host = UnixStream::from_std(host).unwrap();
					let codec_builder = LengthDelimitedCodec::builder();
					let framed = codec_builder.new_framed(host);
					let transport = serde_transport::new(framed, Bincode::default());
					client(transport).await.unwrap()
				},
				pid,
			)),
		}
	}
}
