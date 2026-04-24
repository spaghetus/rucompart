use std::{io::stdin, sync::Arc, time::Duration};

use itertools::Itertools;
use ndarray::Array2;
use rucompart::{Compartment, client_thunk, compartmentalize};
use tarpc::{context::Context, service};
use tokio::{runtime::Runtime, sync::Mutex};

#[derive(Debug, Default)]
struct Game {
	board: [Box<Array2<bool>>; 2],
}

impl From<Array2<bool>> for Game {
	fn from(value: Array2<bool>) -> Self {
		Self {
			board: [Box::new(value.clone()), Box::new(value)],
		}
	}
}

impl Game {
	pub fn tick(&mut self) {
		let [left, right] = &mut self.board;
		right.indexed_iter_mut().for_each(|((y, x), v)| {
			let count = (-1isize..=1)
				.cartesian_product(-1isize..=1)
				.filter(|p| *p != (0, 0))
				.filter_map(|(dy, dx)| Some((y.checked_add_signed(dy)?, x.checked_add_signed(dx)?)))
				.map(|p| left.get(p).map(|v| *v as u8).unwrap_or(0))
				.sum();
			*v = match (left[(y, x)], count) {
				(true, ..2) => false,
				(true, 2..=3) => true,
				(true, 4..) => false,
				(false, 3) => true,
				(false, _) => false,
			};
		});
		std::mem::swap(left, right);
	}
	pub fn get(&self) -> &Array2<bool> {
		&self.board[0]
	}
}

#[service]
trait RGame {
	async fn init(state: Array2<bool>);
	async fn tick();
	async fn get() -> Array2<bool>;
}

#[derive(Clone, Default)]
struct RGameImpl(Arc<Mutex<Game>>);

impl RGame for RGameImpl {
	async fn init(self, _context: ::tarpc::context::Context, state: Array2<bool>) -> () {
		*self.0.lock().await = Game::from(state)
	}

	async fn tick(self, _context: ::tarpc::context::Context) -> () {
		self.0.lock().await.tick();
	}

	async fn get(self, _context: ::tarpc::context::Context) -> Array2<bool> {
		self.0.lock().await.get().clone()
	}
}

compartmentalize!(
	"Conway",
	ServeRGame,
	RGameImpl,
	RGameClient,
	async fn setup(
		service: &mut Self::Server,
		mode: rucompart::CompartmentMode,
	) -> Result<_, std::io::Error> {
		Ok(())
	}
);

fn main() {
	let board = ndarray::array![
		[0, 0, 0, 0, 0, 0, 0, 0, 0, 0,],
		[0, 0, 0, 0, 0, 0, 0, 0, 0, 0,],
		[0, 0, 0, 1, 0, 0, 0, 0, 0, 0,],
		[0, 0, 0, 0, 1, 0, 0, 0, 0, 0,],
		[0, 0, 1, 1, 1, 0, 0, 0, 0, 0,],
		[0, 0, 0, 0, 0, 0, 0, 0, 0, 0,],
		[0, 0, 0, 0, 0, 0, 0, 0, 0, 0,],
		[0, 0, 0, 0, 0, 0, 0, 0, 0, 0,],
		[0, 0, 0, 0, 0, 0, 0, 0, 0, 0,],
		[0, 0, 0, 0, 0, 0, 0, 0, 0, 0,],
	]
	.map(|v| *v != 0);

	let client =
		RGameImpl::fork(|| RGameImpl::default().serve(), client_thunk!(RGameClient)).unwrap();

	Runtime::new().unwrap().block_on(async move {
		let client = client.await;
		client.init(Context::current(), board).await.unwrap();

		loop {
			let state = client.get(Context::current()).await.unwrap();
			if !state.iter().any(|v| *v) {
				break;
			}
			println!("{:#?}", state.map(|v| if *v { 'X' } else { ' ' }));
			client.tick(Context::current()).await.unwrap();
			stdin().read_line(&mut String::new()).unwrap();
		}
	});
}
