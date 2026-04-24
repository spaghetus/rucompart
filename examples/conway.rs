use std::{io::stdin, time::Duration};

use itertools::Itertools;
use ndarray::Array2;

#[derive(Debug)]
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

fn main() {
	let mut board = Game::from(
		ndarray::array![
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
		.map(|v| *v != 0),
	);

	while board.get().iter().any(|v| *v) {
		println!("{:#?}", board.get().map(|v| *v as u8));
		board.tick();
		stdin().read_line(&mut String::new()).unwrap();
	}
}
