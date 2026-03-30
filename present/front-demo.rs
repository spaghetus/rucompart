
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
	async fn hello(name: String) -> String;
}
# #[derive(Clone)]
# pub struct SomeCompartmentService;
impl SomeCompartment for SomeCompartmentService {
	async fn hello(self, _context: ::tarpc::context::Context, name: String) -> String {
		format!("Hello {name}! The compartment's pid is {}", unsafe {
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
fn main() -> eyre::Result<()> {
	let compartment_client =
		compartment_connect!(None, SomeCompartmentService, SomeCompartmentClient);
	Runtime::new()?.block_on(async move {
		let compartment_client = compartment_client.await;
		let ctx = Context::current();

		eprintln!("The host's pid is {}, but...", unsafe { libc::getpid() });
		eprintln!(
			"{}",
			compartment_client.hello(ctx, "world".to_string()).await?
		);

		Ok(())
	})
}