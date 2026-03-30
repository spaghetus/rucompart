
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
# use rucompart::{Compartment, compartmentalize};
# use tarpc::{service};
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
fn main() -> eyre::Result<()> {
	println!("Guest process!");
	println!("Spawning compartment listening on 127.0.0.1:1234...");
	SomeCompartmentService.listen_on_tcp(SomeCompartmentService.serve(), "127.0.0.1:1234".parse().unwrap())
}