use clap::{Parser, Subcommand};
use futures::FutureExt;
use rucompart::{Compartment, compartment_connect, compartmentalize};
use std::{os::fd::FromRawFd, str::FromStr};
use tarpc::{context::Context, service};
use tokio::{
	net::{UnixSocket, UnixStream},
	runtime::Runtime,
};

#[service]
pub trait Rootless {
	async fn hello(name: String);
}

#[derive(Clone)]
struct RootlessService;

impl Rootless for RootlessService {
	async fn hello(self, _context: ::tarpc::context::Context, name: String) -> () {
		println!("Hello {name}! The compartment's uid is {}", unsafe {
			libc::getuid()
		});
	}
}

compartmentalize!(
	"ROOTLESS",
	ServeRootless,
	RootlessService,
	RootlessClient,
	async fn setup(&mut self, mode: rucompart::CompartmentMode) -> Result<_, std::io::Error> {
		unsafe {
			libc::setuid(65534);
			Ok(())
		}
	}
);

// I'd like to automate the argument-parsing part, though with how clap's
// syntax works it might be a little troublesome.
#[derive(Subcommand)]
enum Cmd {
	Rootless {
		socket_address: String,
	},
	Main {
		#[arg(long)]
		rootless_addr: Option<String>,
	},
}

#[derive(Parser)]
struct Args {
	#[command(subcommand)]
	cmd: Cmd,
}

fn main() {
	let Args { cmd } = Args::parse();
	if let Cmd::Rootless { socket_address } = cmd {
		if socket_address == "systemd" {
			let fd = std::env::var("LISTEN_FDS").unwrap().parse().unwrap();
			let stream = unsafe { std::os::unix::net::UnixStream::from_raw_fd(fd) };
			stream.set_nonblocking(true).unwrap();
			let stream = UnixStream::from_std(stream).unwrap();
			Runtime::new()
				.unwrap()
				.block_on(RootlessService.listen_on_stream(RootlessService.serve(), stream));
			unreachable!()
		} else if let Ok(addr) = std::net::SocketAddr::from_str(&socket_address) {
			RootlessService.listen_on_tcp(RootlessService.serve(), addr)
		} else {
			RootlessService.listen_on_unix(RootlessService.serve(), socket_address)
		}
	};
	let Cmd::Main { rootless_addr } = cmd else {
		unreachable!()
	};
	let rootless_client = compartment_connect!(rootless_addr, RootlessService, RootlessClient);
	Runtime::new().unwrap().block_on(async move {
		println!("The host's uid is {}, but...", unsafe { libc::getuid() });
		let rootless_client: RootlessClient = rootless_client.await;
		rootless_client
			.hello(Context::current(), "world".into())
			.await
			.unwrap();
	});
}
