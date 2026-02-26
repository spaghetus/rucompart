use serde::{Deserialize, Serialize};
use std::{convert::Infallible, error::Error};
use tarpc::{
	serde_transport::{self, Transport as STransport},
	server::{BaseChannel, Serve},
	tokio_serde::formats::Bincode,
	tokio_util::codec::LengthDelimitedCodec,
};
use tokio::{
	net::UnixStream,
	runtime::{Handle, Runtime},
	task::JoinHandle,
};

pub enum CompartmentMode {
	Fork,
	StandaloneSock,
}

#[macro_export]
macro_rules! compartmentalize {
	($(:name $env_name:literal,)?$($serve:ident)::+, $service:ty, $client:ty, async fn setup(&self, $mname:ident: $(rucompart::)?CompartmentMode) -> Result<$_:ty, $setup_err:ty> $setup:tt) => {
		impl rucompart::Compartment for $service {
			$(
				#[cfg(feature = "standalone")]
				const ENV_PREFIX: &str = $env_name;
			)?
			type Error = $setup_err;

			type Server = $($serve)::+<$service>;

			type Client = $client;

			fn setup(
				&self,
				$mname: rucompart::CompartmentMode
			) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + Sync {
				async { $setup }
			}

			fn serve(
				server: Self::Server,
				channel: rucompart::CompartmentChannel<Self>,
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

pub type CompartmentChannel<C> = BaseChannel<
	<<C as Compartment>::Server as Serve>::Req,
	<<C as Compartment>::Server as Serve>::Resp,
	STransport<
		UnixStream,
		tarpc::ClientMessage<<<C as Compartment>::Server as Serve>::Req>,
		tarpc::Response<<<C as Compartment>::Server as Serve>::Resp>,
		Bincode<
			tarpc::ClientMessage<<<C as Compartment>::Server as Serve>::Req>,
			tarpc::Response<<<C as Compartment>::Server as Serve>::Resp>,
		>,
	>,
>;

pub type CompartmentTransport<C> = STransport<
	UnixStream,
	tarpc::Response<<<C as Compartment>::Server as Serve>::Resp>,
	tarpc::ClientMessage<<<C as Compartment>::Server as Serve>::Req>,
	Bincode<
		tarpc::Response<<<C as Compartment>::Server as Serve>::Resp>,
		tarpc::ClientMessage<<<C as Compartment>::Server as Serve>::Req>,
	>,
>;

pub trait Compartment: Sized + Send + Sync {
	#[cfg(feature = "standalone")]
	const ENV_PREFIX: &str;

	type Error: Error + From<std::io::Error>;
	fn setup(
		&self,
		mode: CompartmentMode,
	) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + Sync;

	fn serve(
		server: Self::Server,
		channel: CompartmentChannel<Self>,
	) -> impl Future<Output = ()> + Send + Sync;

	type Server: Serve + Clone + Send + Sync;
	type Client: Send + Sync + 'static;

	fn fork(
		self,
		server: impl FnOnce() -> Self::Server + Send + Sync,
		client: impl FnOnce(CompartmentTransport<Self>) -> JoinHandle<Self::Client>,
	) -> Result<impl Future<Output = Self::Client>, Self::Error>
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
				self.setup(CompartmentMode::Fork).await.unwrap();
				Self::serve(server(), channel).await;
				std::process::exit(0)
			}),
			_ => Ok(async move {
				std::mem::forget(guest);
				let host = UnixStream::from_std(host).unwrap();
				let codec_builder = LengthDelimitedCodec::builder();
				let framed = codec_builder.new_framed(host);
				let transport = serde_transport::new(framed, Bincode::default());
				client(transport).await.unwrap()
			}),
		}
	}

	/// Check whether we're called as a standalone instance of this compartment; if so, this function never returns.
	#[cfg(feature = "standalone")]
	fn standalone(
		&self,
		server: impl FnOnce() -> Self::Server + Send + Sync,
		client: impl FnOnce(CompartmentTransport<Self>) -> JoinHandle<Self::Client>,
		serve: bool,
	) -> Result<impl Future<Output = Self::Client>, Infallible>
	where
		<Self::Server as Serve>::Req: Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static,
		<Self::Server as Serve>::Resp:
			Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static,
	{
		use std::{path::PathBuf, str::FromStr};
		let env_name = format!(
			"{}_{}_ADDR",
			env!("CARGO_PKG_NAME").to_ascii_uppercase(),
			Self::ENV_PREFIX
		);
		let path = std::env::var(env_name).expect("Socket address variable is missing");
		// If the environment variable *isn't set*, we return None
		// (a zero-sized type, because the Some arm of the enum contains a bottom type and can never exist)
		if serve {
			// systemd_socket::init().expect("Failed to initialize systemd sockets");
			// let socket_addr = systemd_socket::SocketAddr::from_str(path).unwrap();
			Runtime::new().unwrap().block_on(async move {
				use futures::StreamExt;
				use tarpc::serde_transport::unix::listen_on;
				use tokio::net::UnixListener;
				let listener = UnixListener::bind(path).unwrap();
				let server = server();
				listen_on(listener, Bincode::default)
					.await
					.unwrap()
					.filter_map(|r| async { r.ok() })
					.for_each(move |transport| {
						let server = server.clone();
						async move {
							let channel = BaseChannel::with_defaults(transport);
							Self::serve(server.clone(), channel).await;
						}
					})
					.await;
			});
			unreachable!()
		} else {
			Ok(async move {
				let stream = UnixStream::connect(path).await.unwrap();
				let codec_builder = LengthDelimitedCodec::builder();
				let framed = codec_builder.new_framed(stream);
				let transport = serde_transport::new(framed, Bincode::default());
				client(transport).await.unwrap()
			})
		}
	}
}
