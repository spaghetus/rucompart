
# //! ```cargo
# //! [dependencies]
# //! tokio = { version = "1.49.0", features = ["macros", "net", "rt-multi-thread"] }
# //! tarpc = { version = "0.37.0", features = [
# //!     "serde-transport",
# //!     "serde-transport-bincode",
# //!     "unix",
# //!     "tcp",
# //! ] }
# //! libc = "0.2.181"
# //! futures = "0.3.31"
# //! eyre = "0.6.12"
# //! rucompart = {git = "https://github.com/spaghetus/rucompart"}
# //! ```
# use rucompart::{compartment_connect, compartmentalize};
# use tarpc::{context::Context, service};
# use tokio::runtime::Runtime;
// Some boilerplate elided
#[service]
pub trait SomeCompartment {
	async fn hello(name: String);
}
# #[derive(Clone)]
# pub struct SomeCompartmentService;
impl SomeCompartment for SomeCompartmentService {
	async fn hello(self, _context: ::tarpc::context::Context, name: String) {
		println!("Hello {name}! The compartment's pid is {}", unsafe {
			libc::getpid()
		})
	}
}
# compartmentalize!(
# 	"SOME_COMPARTMENT",
# 	ServeSomeCompartment,
# 	SomeCompartmentService,
# 	SomeCompartmentClient,
# 	async fn setup(&mut self, _mode: rucompart::CompartmentMode) -> Result<_, std::io::Error> {
# 		Ok(())
# 	}
# );
fn main() {
	println!("Host process!");
	std::thread::sleep(std::time::Duration::from_secs(3));
	println!("Connecting...");
	let client = compartment_connect!(Some("127.0.0.1:1234".to_string()), SomeCompartmentService, SomeCompartmentClient);
	Runtime::new().unwrap().block_on(async move {
		let client = client.await;
		println!("Connected!");

		eprintln!("The host's pid is {}, but...", unsafe { libc::getpid() });
		client.hello(Context::current(), "world".to_string()).await.unwrap();
		std::thread::sleep(std::time::Duration::from_secs(3));
		std::mem::drop(client);
	});
}