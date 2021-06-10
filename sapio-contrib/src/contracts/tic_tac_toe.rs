// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! contracts for playing a version of tic-tac-toe
use sapio::contract::actions::ConditionalCompileType;
use sapio::contract::*;
use sapio::template::Template;
use sapio::*;
use sapio_base::timelocks::RelHeight;
use schemars::*;
use serde::*;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

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

#[derive(Clone, Serialize, Deserialize, JsonSchema, Hash, Eq, PartialEq, Copy)]
struct Board([[Option<Tile>; 3]; 3]);

impl Board {
    fn winner(&self) -> Option<Tile> {
        for tile in [Some(Tile::X), Some(Tile::O)].into_iter() {
            for i in 0..3 {
                if self.0[i].iter().all(|t| *t == *tile) {
                    return *tile;
                }
            }
            for j in 0..3 {
                if self.0.iter().all(|t| t[j] == *tile) {
                    return *tile;
                }
            }
            if self.0[1][1] == *tile {
                if self.0[0][0] == self.0[1][1] && self.0[2][2] == self.0[0][0] {
                    return *tile;
                }

                if self.0[2][0] == self.0[1][1] && self.0[2][2] == self.0[0][2] {
                    return *tile;
                }
            }
        }
        None
    }
}

/// TicTacToe Game Contract
#[derive(Clone)]
pub struct TicTacToe {
    board: Board,
    whose_turn: Tile,
    win_key_x: Arc<dyn Compilable>,
    win_key_o: Arc<dyn Compilable>,
    cache: Arc<Mutex<HashMap<(&'static str, Board, Tile), Vec<Template>>>>,
}

impl TicTacToe {
    compile_if! {
        fn no_winner(self, _ctx) {
            if self.board.winner().is_none() {
                ConditionalCompileType::Required
            } else {
                ConditionalCompileType::Never
            }
        }
    }

    compile_if! {
        fn winner(self, _ctx) {
            if self.board.winner().is_none() {
                ConditionalCompileType::Never
            } else {
                ConditionalCompileType::Required
            }
        }
    }
    then! {
        compile_if: [Self::no_winner]
        fn make_move(self, ctx) {
            loop {

                if let Some(entry) = self.cache.lock().unwrap().get(&("make_move", self.board, self.whose_turn)) {
                    return Ok(Box::new(entry.clone().into_iter().map(Ok)));
                } else {

                    let mut v = vec![];
                    for i in 0..3 {
                        for j in 0..3 {
                            if let None = self.board.0[i][j] {
                                let mut bcopy = self.board.clone();
                                bcopy.0[i][j] = Some(self.whose_turn);
                                let tmpl = ctx.template()
                                            .add_output(ctx.funds(),
                                                        &TicTacToe { board:bcopy,
                                                                    whose_turn: self.whose_turn.next(),
                                                                    ..self.clone()},
                                                        None)?.into();
                                v.push(tmpl);
                            }
                        }
                    }
                    let mut g = self.cache.lock().unwrap();
                    g.insert(("make_move", self.board, self.whose_turn), v);
                }
            }
        }
    }

    then! {
        compile_if: [Self::winner]
        fn claim_winnings(self, ctx) {
            let winner = self.board.winner().unwrap();
            match winner {
                Tile::X => {
                  ctx.template().add_output(ctx.funds(), &*self.win_key_x, None)?.into()
                }
                Tile::O => {
                  ctx.template().add_output(ctx.funds(), &*self.win_key_o, None)?.into()
                }
            }
        }
    }

    then! {
        compile_if: [Self::no_winner]
        fn timeout(self, ctx) {
            let defaults_to = self.whose_turn.next();
            match defaults_to {
                Tile::X => {
                  ctx.template().add_output(ctx.funds(), &*self.win_key_x, None)?
                  .set_sequence(0, RelHeight::from(144).into())?.into()
                }
                Tile::O => {
                  ctx.template().add_output(ctx.funds(), &*self.win_key_o, None)?
                  .set_sequence(0, RelHeight::from(144).into())?.into()
                }
            }
        }
    }
}

impl Contract for TicTacToe {
    declare! {then, Self::make_move, Self::claim_winnings}
    declare! {non updatable}
}
