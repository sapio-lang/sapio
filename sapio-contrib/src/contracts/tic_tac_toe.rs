// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! contracts for playing a version of tic-tac-toe
use sapio::contract::actions::ConditionalCompileType;
use sapio::contract::*;
use sapio::*;
use sapio_base::timelocks::RelHeight;
use sapio_macros::compile_if;

use schemars::*;
use serde::*;
use std::sync::Arc;

#[derive(Clone, Serialize, Eq, PartialEq, Ord, PartialOrd, Copy, Deserialize, JsonSchema, Hash)]
enum Tile {
    X,
    O,
}
impl Tile {
    fn next(&self) -> Self {
        match self {
            Tile::X => Tile::O,
            Tile::O => Tile::X,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, JsonSchema, Hash, Eq, PartialEq, Copy, PartialOrd, Ord)]
struct Board([[Option<Tile>; 3]; 3]);

impl Board {
    fn wins(&self, tile: Tile) -> bool {
        let tile = Some(tile);
        (0..3).any(|i| {
            self.0[i].iter().all(|t| *t == tile) || self.0.iter().all(|row| row[i] == tile)
        }) || (0..3).all(|i| self.0[i][i] == tile)
            || (0..3).all(|i| self.0[i][2 - i] == tile)
    }

    fn full(&self) -> bool {
        self.0.iter().flatten().all(Option::is_some)
    }

    fn winner(&self) -> Option<Tile> {
        [Tile::X, Tile::O].into_iter().find(|tile| self.wins(*tile))
    }
}

/// TicTacToe Game Contract
#[derive(Clone)]
pub struct TicTacToe {
    board: Board,
    whose_turn: Tile,
    win_key_x: Arc<dyn Compilable>,
    win_key_o: Arc<dyn Compilable>,
    move_key_x: bitcoin::XOnlyPublicKey,
    move_key_o: bitcoin::XOnlyPublicKey,
}

impl TicTacToe {
    /// Start an empty game with separate move keys and payout contracts.
    /// A draw splits the balance equally, with an odd satoshi assigned to O.
    pub fn new(
        move_key_x: bitcoin::XOnlyPublicKey,
        move_key_o: bitcoin::XOnlyPublicKey,
        win_key_x: Arc<dyn Compilable>,
        win_key_o: Arc<dyn Compilable>,
    ) -> Self {
        Self {
            board: Board([[None; 3]; 3]),
            whose_turn: Tile::X,
            win_key_x,
            win_key_o,
            move_key_x,
            move_key_o,
        }
    }

    #[compile_if]
    fn no_winner(self, _ctx: Context) {
        if self.board.winner().is_none() && !self.board.full() {
            ConditionalCompileType::Required
        } else {
            ConditionalCompileType::Never
        }
    }

    #[compile_if]
    fn winner(self, _ctx: Context) {
        if self.board.winner().is_none() {
            ConditionalCompileType::Never
        } else {
            ConditionalCompileType::Required
        }
    }
    #[guard]
    fn current_player(self, _ctx: Context) {
        sapio_base::Clause::Key(match self.whose_turn {
            Tile::X => self.move_key_x,
            Tile::O => self.move_key_o,
        })
    }

    #[then(
        compile_if = "[Self::no_winner]",
        guarded_by = "[Self::current_player]"
    )]
    fn make_move(self, ctx: sapio::Context) {
        let mut ctx = ctx;
        // Templates contain paths, amounts and effects from this invocation;
        // reusing them by board position alone would change the contract.
        let mut v = vec![];
        for i in 0..3 {
            let mut i_ctx = ctx.derive_num(i as u64)?;
            for j in 0..3 {
                if self.board.0[i][j].is_none() {
                    let j_ctx = i_ctx.derive_num(j as u64)?;
                    let mut bcopy = self.board;
                    bcopy.0[i][j] = Some(self.whose_turn);
                    let tmpl = j_ctx
                        .template()
                        .add_output(
                            ctx.funds(),
                            &TicTacToe {
                                board: bcopy,
                                whose_turn: self.whose_turn.next(),
                                ..self.clone()
                            },
                            None,
                        )?
                        .into();
                    v.push(tmpl);
                }
            }
        }
        Ok(Box::new(v.into_iter().map(Ok)))
    }

    #[then(compile_if = "[Self::winner]")]
    fn claim_winnings(self, ctx: sapio::Context) {
        let winner = self
            .board
            .winner()
            .ok_or(CompilationError::TerminateCompilation)?;
        let f = ctx.funds();
        match winner {
            Tile::X => ctx.template().add_output(f, &*self.win_key_x, None)?.into(),
            Tile::O => ctx.template().add_output(f, &*self.win_key_o, None)?.into(),
        }
    }

    #[then(compile_if = "[Self::no_winner]")]
    fn timeout(self, ctx: sapio::Context) {
        let defaults_to = self.whose_turn.next();
        let f = ctx.funds();
        match defaults_to {
            Tile::X => ctx
                .template()
                .add_output(f, &*self.win_key_x, None)?
                .set_sequence(0, RelHeight::from(144).into())?
                .into(),
            Tile::O => ctx
                .template()
                .add_output(f, &*self.win_key_o, None)?
                .set_sequence(0, RelHeight::from(144).into())?
                .into(),
        }
    }

    #[compile_if]
    fn drawn(self, _ctx: Context) {
        if self.board.full() && self.board.winner().is_none() {
            ConditionalCompileType::Required
        } else {
            ConditionalCompileType::Never
        }
    }

    #[then(compile_if = "[Self::drawn]")]
    fn refund_draw(self, ctx: Context) {
        let half = ctx.funds() / 2;
        let remainder = ctx.funds() - half;
        ctx.template()
            .add_output(half, &*self.win_key_x, None)?
            .add_output(remainder, &*self.win_key_o, None)?
            .into()
    }
}

impl Contract for TicTacToe {
    declare! {actions, Self::make_move, Self::claim_winnings, Self::timeout, Self::refund_draw}

    fn ensure_amount(&self, ctx: Context) -> Result<bitcoin::Amount, CompilationError> {
        let x = self
            .board
            .0
            .iter()
            .flatten()
            .filter(|tile| **tile == Some(Tile::X))
            .count();
        let o = self
            .board
            .0
            .iter()
            .flatten()
            .filter(|tile| **tile == Some(Tile::O))
            .count();
        let valid_turn =
            (x == o && self.whose_turn == Tile::X) || (x == o + 1 && self.whose_turn == Tile::O);
        if !valid_turn
            || (self.board.wins(Tile::X) && (x != o + 1 || self.board.wins(Tile::O)))
            || (self.board.wins(Tile::O) && x != o)
            || self.move_key_x == self.move_key_o
        {
            return Err(CompilationError::Custom(
                "Invalid TicTacToe state or player keys".into(),
            ));
        }
        if ctx.funds().to_sat() < 2 {
            return Err(CompilationError::OutOfFunds);
        }
        Ok(ctx.funds())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{context, key};

    fn make_game(board: Board, whose_turn: Tile) -> TicTacToe {
        TicTacToe {
            board,
            whose_turn,
            win_key_x: Arc::new(key(1)),
            win_key_o: Arc::new(key(2)),
            move_key_x: key(1),
            move_key_o: key(2),
        }
    }

    #[test]
    fn every_winning_line_is_detected_without_false_diagonals() {
        let initial = TicTacToe::new(key(1), key(2), Arc::new(key(1)), Arc::new(key(2)));
        assert!(initial.board.winner().is_none() && !initial.board.full());
        assert_eq!(
            initial.guard_current_player(context(0)),
            sapio_base::Clause::Key(key(1))
        );
        for line in [
            [0, 1, 2],
            [3, 4, 5],
            [6, 7, 8],
            [0, 3, 6],
            [1, 4, 7],
            [2, 5, 8],
            [0, 4, 8],
            [2, 4, 6],
        ] {
            for tile in [Tile::X, Tile::O] {
                let mut board = Board([[None; 3]; 3]);
                for i in line {
                    board.0[i / 3][i % 3] = Some(tile);
                }
                assert!(board.winner() == Some(tile));
            }
        }
        let board = Board([
            [None, None, Some(Tile::O)],
            [None, Some(Tile::X), None],
            [Some(Tile::X), None, Some(Tile::O)],
        ]);
        assert!(board.winner().is_none());
    }

    #[test]
    fn draw_splits_the_entire_balance_and_has_no_moves() {
        use Tile::*;
        let game = make_game(
            Board([
                [Some(X), Some(O), Some(X)],
                [Some(X), Some(O), Some(O)],
                [Some(O), Some(X), Some(X)],
            ]),
            O,
        );
        let object = game.compile(context(1001)).unwrap();
        object.validate().unwrap();
        assert_eq!(object.ctv_to_tx.len(), 1);
        assert_eq!(
            object
                .ctv_to_tx
                .values()
                .next()
                .unwrap()
                .tx
                .output
                .iter()
                .map(|o| o.value.to_sat())
                .collect::<Vec<_>>(),
            vec![500, 501]
        );
    }

    #[test]
    fn final_move_is_signed_timeout_is_available_and_templates_use_current_funds() {
        use Tile::*;
        let game = make_game(
            Board([
                [Some(X), Some(O), Some(X)],
                [Some(X), Some(O), Some(O)],
                [Some(O), Some(X), None],
            ]),
            X,
        );
        assert_eq!(
            game.guard_current_player(context(0)),
            sapio_base::Clause::Key(key(1))
        );
        assert_eq!(TicTacToe::make_move().unwrap().get_guard().len(), 1);
        for funds in [1000, 2000] {
            let object = game.compile(context(funds)).unwrap();
            object.validate().unwrap();
            assert_eq!(object.ctv_to_tx.len(), 2);
            assert!(object
                .ctv_to_tx
                .values()
                .all(|t| t.total_amount().to_sat() == funds));
            assert!(object
                .ctv_to_tx
                .values()
                .any(|t| t.tx.input[0].sequence.to_consensus_u32() == 144));
        }
        assert!(make_game(Board([[None; 3]; 3]), O)
            .compile(context(1000))
            .is_err());
        let mut duplicate_keys = make_game(Board([[None; 3]; 3]), X);
        duplicate_keys.move_key_o = duplicate_keys.move_key_x;
        assert!(duplicate_keys.compile(context(1000)).is_err());
    }
}
