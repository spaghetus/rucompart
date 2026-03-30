//! ```cargo
//! [dependencies]
//! tokio = { version = "1.49.0", features = ["macros", "net", "rt-multi-thread"] }
//! ```

#[macro_use]
extern crate tarpc;
#[macro_use]
extern crate rucompart;
use rucompart::{Compartment, compartment_connect, compartmentalize};
use tarpc::{context::Context, service};

/// API surface
#[service]
pub trait SomeCompartment {
	async fn hello(name: String) -> String;

	async fn sum(a: i64, b: i64) -> i64;

	async fn sort(list: Vec<i64>) -> Vec<i64>;
}

/// Marker zero-size-type for a particular implementation of the service
#[derive(Clone)]
pub struct SomeCompartmentService;

/// The actual implementation
impl SomeCompartment for SomeCompartmentService {
	async fn hello(self, _context: ::tarpc::context::Context, name: String) -> String {
		format!("Hello {name}! The compartment's pid is {}", unsafe {
			libc::getpid()
		})
	}

	async fn sum(self, _context: ::tarpc::context::Context, a: i64, b: i64) -> i64 {
		a + b
	}

	async fn sort(self, _context: ::tarpc::context::Context, mut list: Vec<i64>) -> Vec<i64> {
		list.sort();
		list
	}
}

// A macro to do some annoying boilerplate:
// In theory, it should be possible to do this with generics,
// but due to some intricacies of the type system implementation, it is not.
compartmentalize!(
	"SOME_COMPARTMENT",
	ServeSomeCompartment,
	SomeCompartmentService,
	SomeCompartmentClient,
	async fn setup(&mut self, mode: rucompart::CompartmentMode) -> Result<_, std::io::Error> {
		Ok(())
	}
);
use tokio::runtime::Runtime;

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
