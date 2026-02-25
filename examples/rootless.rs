use rucompart::{Compartment, compartmentalize};
use tarpc::{client::Config, context::Context, service};
use tokio::runtime::Runtime;

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
	ServeRootless,
	RootlessService,
	RootlessClient,
	async fn setup(&self) -> Result<_, std::io::Error> {
		unsafe {
			libc::setuid(65534);
			Ok(())
		}
	}
);

fn main() {
	let (client, pid) = RootlessService
		.spawn(
			|| RootlessService.serve(),
			|transport| {
				tokio::spawn(async { RootlessClient::new(Config::default(), transport).spawn() })
			},
		)
		.unwrap();
	Runtime::new().unwrap().block_on(async move {
		println!("The host's uid is {}, but...", unsafe { libc::getuid() });
		let client: RootlessClient = client.await;
		client
			.hello(Context::current(), "world".into())
			.await
			.unwrap();
	});
}
