pub mod chan;

use serde::{Deserialize, Serialize};
use std::{error::Error, net::SocketAddr, path::Path};
use tarpc::{
	serde_transport::{self, Transport as STransport},
	server::{BaseChannel, Serve},
	tokio_serde::formats::Json,
	tokio_util::codec::LengthDelimitedCodec,
};
use tokio::{
	io::{AsyncRead, AsyncWrite},
	net::{TcpListener, TcpStream, UnixStream},
	runtime::{Handle, Runtime},
	task::JoinHandle,
};

pub enum CompartmentMode {
	Fork,
	StandaloneSock,
}

#[macro_export]
macro_rules! compartmentalize {
	($env_name:literal, $($serve:ident)::+, $service:ty, $client:ty, async fn setup($sname:ident: &mut Self::Server, $mname:ident: $(rucompart::)?CompartmentMode$(,)?) -> Result<$_:ty, $setup_err:ty> $setup:tt) => {
		impl rucompart::Compartment for $service {
			const ENV_PREFIX: &str = $env_name;
			type Error = $setup_err;

			type Server = $($serve)::+<$service>;

			type Client = $client;

			fn setup(
				$sname: &mut Self::Server,
				$mname: rucompart::CompartmentMode
			) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + Sync {
				async { $setup }
			}

			fn serve<STR: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Sync>(
				server: Self::Server,
				channel: rucompart::CompartmentChannel<Self, STR>,
			) -> impl std::future::Future<Output = ()> + Send + Sync {
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

#[macro_export]
macro_rules! compartment_connect {
	($addr:expr, $service:ty, $server:expr, $client:ty) => {{
		use futures::FutureExt;
		use rucompart::Compartment;
		let addr: Option<String> = $addr;
		addr.map(|addr| {
			use std::str::FromStr;
			if let Ok(addr) = std::net::SocketAddr::from_str(&addr) {
				<$service>::connect_to_tcp(
					|transport| {
						tokio::spawn(async {
							<$client>::new(tarpc::client::Config::default(), transport).spawn()
						})
					},
					addr,
				)
				.boxed()
			} else {
				<$service>::connect_to_unix(
					|transport| {
						tokio::spawn(async {
							<$client>::new(tarpc::client::Config::default(), transport).spawn()
						})
					},
					addr,
				)
				.boxed()
			}
		})
		.unwrap_or_else(|| {
			<$service>::fork(
				|| ($server)(),
				|transport| {
					tokio::spawn(async {
						<$client>::new(tarpc::client::Config::default(), transport).spawn()
					})
				},
			)
			.unwrap()
			.boxed()
		})
	}};
}

pub type CompartmentChannel<C, STR> = BaseChannel<
	<<C as Compartment>::Server as Serve>::Req,
	<<C as Compartment>::Server as Serve>::Resp,
	STransport<
		STR,
		tarpc::ClientMessage<<<C as Compartment>::Server as Serve>::Req>,
		tarpc::Response<<<C as Compartment>::Server as Serve>::Resp>,
		Json<
			tarpc::ClientMessage<<<C as Compartment>::Server as Serve>::Req>,
			tarpc::Response<<<C as Compartment>::Server as Serve>::Resp>,
		>,
	>,
>;

pub type CompartmentTransport<C, STR> = STransport<
	STR,
	tarpc::Response<<<C as Compartment>::Server as Serve>::Resp>,
	tarpc::ClientMessage<<<C as Compartment>::Server as Serve>::Req>,
	Json<
		tarpc::Response<<<C as Compartment>::Server as Serve>::Resp>,
		tarpc::ClientMessage<<<C as Compartment>::Server as Serve>::Req>,
	>,
>;

pub trait Compartment: Sized + Send + Sync {
	const ENV_PREFIX: &str;

	type Error: Error + From<std::io::Error>;
	fn setup(
		server: &mut Self::Server,
		mode: CompartmentMode,
	) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + Sync;

	fn serve<STR: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Sync>(
		server: Self::Server,
		channel: CompartmentChannel<Self, STR>,
	) -> impl Future<Output = ()> + Send + Sync;

	type Server: Serve + Clone + Send + Sync;
	type Client: Send + Sync + 'static;

	fn fork(
		server: impl FnOnce() -> Self::Server + Send + Sync,
		client: impl FnOnce(CompartmentTransport<Self, UnixStream>) -> JoinHandle<Self::Client>,
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
				let transport = tarpc::serde_transport::new(framed, Json::default());
				let channel = BaseChannel::with_defaults(transport);
				let mut server = server();
				Self::setup(&mut server, CompartmentMode::Fork)
					.await
					.unwrap();
				Self::serve(server, channel).await;
				std::process::exit(0)
			}),
			_ => Ok(async move {
				std::mem::forget(guest);
				let host = UnixStream::from_std(host).unwrap();
				let codec_builder = LengthDelimitedCodec::builder();
				let framed = codec_builder.new_framed(host);
				let transport = serde_transport::new(framed, Json::default());
				client(transport).await.unwrap()
			}),
		}
	}

	/// Check whether we're called as a standalone instance of this compartment; if so, this function never returns.
	#[deprecated]
	#[cfg(feature = "standalone")]
	fn standalone(
		&self,
		server: impl FnOnce() -> Self::Server + Send + Sync,
		client: impl FnOnce(CompartmentTransport<Self, UnixStream>) -> JoinHandle<Self::Client>,
		serve: bool,
	) -> Result<impl Future<Output = Self::Client>, std::convert::Infallible>
	where
		<Self::Server as Serve>::Req: Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static,
		<Self::Server as Serve>::Resp:
			Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static,
	{
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
				listen_on(listener, Json::default)
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
				let transport = serde_transport::new(framed, Json::default());
				client(transport).await.unwrap()
			})
		}
	}

	fn connect_to_stream<STR: AsyncRead + AsyncWrite + Send + Sync>(
		client: impl FnOnce(CompartmentTransport<Self, STR>) -> JoinHandle<Self::Client>,
		stream: STR,
	) -> impl Future<Output = Self::Client>
	where
		<Self::Server as Serve>::Req: Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static,
		<Self::Server as Serve>::Resp:
			Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static,
	{
		async move {
			let codec_builder = LengthDelimitedCodec::builder();
			let framed = codec_builder.new_framed(stream);
			let transport = serde_transport::new(framed, Json::default());
			client(transport).await.unwrap()
		}
	}

	fn connect_to_unix(
		client: impl FnOnce(CompartmentTransport<Self, UnixStream>) -> JoinHandle<Self::Client>,
		addr: impl AsRef<Path>,
	) -> impl Future<Output = Self::Client>
	where
		<Self::Server as Serve>::Req: Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static,
		<Self::Server as Serve>::Resp:
			Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static,
	{
		async move {
			let stream = UnixStream::connect(addr).await.unwrap();
			Self::connect_to_stream(client, stream).await
		}
	}

	fn connect_to_tcp(
		client: impl FnOnce(CompartmentTransport<Self, TcpStream>) -> JoinHandle<Self::Client>,
		addr: SocketAddr,
	) -> impl Future<Output = Self::Client>
	where
		<Self::Server as Serve>::Req: Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static,
		<Self::Server as Serve>::Resp:
			Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static,
	{
		async move {
			let stream = TcpStream::connect(addr).await.unwrap();
			Self::connect_to_stream(client, stream).await
		}
	}

	/// Run a standalone compartment, listening on a certain stream.
	fn listen_on_stream(
		&self,
		server: Self::Server,
		stream: impl AsyncRead + AsyncWrite + Send + Sync,
	) -> impl std::future::Future<Output = ()> + Send
	where
		<Self::Server as Serve>::Req: Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static,
		<Self::Server as Serve>::Resp:
			Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static,
	{
		async {
			let codec_builder = LengthDelimitedCodec::builder();
			let framed = codec_builder.new_framed(stream);
			let transport = tarpc::serde_transport::new(framed, Json::default());
			let channel = BaseChannel::with_defaults(transport);
			Self::serve(server, channel).await;
			std::process::exit(0)
		}
	}

	/// Run a standalone compartment, listening on a certain unix socket.
	fn listen_on_unix(&self, server: Self::Server, path: impl AsRef<Path>) -> !
	where
		<Self::Server as Serve>::Req: Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static,
		<Self::Server as Serve>::Resp:
			Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static,
	{
		Runtime::new().unwrap().block_on(async move {
			use tokio::net::UnixListener;
			let listener = UnixListener::bind(path).unwrap();
			let (stream, _) = listener.accept().await.unwrap();
			self.listen_on_stream(server, stream).await;
		});
		unreachable!()
	}

	/// Run a standalone compartment, listening on a certain TCP socket.
	fn listen_on_tcp(&self, server: Self::Server, addr: SocketAddr) -> !
	where
		<Self::Server as Serve>::Req: Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static,
		<Self::Server as Serve>::Resp:
			Serialize + for<'de> Deserialize<'de> + Send + Sync + 'static,
	{
		Runtime::new().unwrap().block_on(async move {
			let listener = TcpListener::bind(addr).await.unwrap();
			let (stream, _) = listener.accept().await.unwrap();
			self.listen_on_stream(server, stream).await;
		});
		unreachable!()
	}
}
