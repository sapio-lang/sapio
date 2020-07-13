import os
from sapio_compiler import (
    AbsoluteTimeSpec,
    Days,
    RelativeTimeSpec,
    TimeSpec,
    Weeks,
    AmountRange,
)

from sapio_bitcoinlib.static_types import Bitcoin, PubKey, Amount
from sapio_zoo.p2pk import PayToPubKey

from sapio_compiler import *
from typing import Tuple, Optional, Any
from functools import lru_cache


Board = int


def NewBoard():
    # 9*2 digits long
    return 0b1_000_000_000_000_000_000


def all_filled(board: Board) -> bool:
    return (board & 0b111111111) | ((board >> 9) & 0b111111111) == 0b111111111


winning_patterns = [
    0b111_000_000,
    0b000_111_000,
    0b000_000_111,
    0b100_100_100,
    0b010_010_010,
    0b001_001_001,
    0b100_010_001,
    0b001_010_100,
]


def winner(board: Board) -> Optional[bool]:
    for pat in winning_patterns:
        if pat & (board & 0b111111111) == pat:
            return False
        if pat & ((board >> 9) & 0b111111111) == pat:
            return True
    return None


cache = {}


def TicTacToeState(board: Board, player: bool):
    if board in cache:
        return cache[board]
    board_won = winner(board)
    unwinnable = board_won is None and all_filled(board)
    shift = 9*player
    filled = board | (board >> 9)
    moves = [board | ((1 << j) << (shift)) for j in range(9) if filled & (1 << j) == 0]

    class TicTacToe(Contract):
        class Fields:
            amount: Amount
            player_1: PubKey
            player_2: PubKey
        # declare the checks for signatures from players

        @require
        def player_one(self):
            return SignedBy(self.player_1)

        @require
        def player_two(self):
            return SignedBy(self.player_2)

        @require
        def current_player(self):
            return SignedBy(self.player_2) if player else SignedBy(self.player_1)

        @require
        def next_player(self):
            return SignedBy(self.player_2) if not player else SignedBy(self.player_1)

        # Player 1 wins, only enabled for winning=True boards
        if board_won is True:
            @player_one
            @unlock
            def player_1_wins(self):
                return Satisfied()

        # Player 2 wins, only enabled for winning=False boards
        if board_won is False:
            @player_two
            @unlock
            def player_2_wins(self):
                return Satisfied()

        # Only enable other cases when a winner is not yet picked.
        if board_won is None and not unwinnable:
            # The current player has a week to select and broadcast their move
            # between when the anymove branch is available.
            #
            # Either a week goes by, then their turn can be finalized,
            # or via the coop path the signed tx gets accepted.
            #
            # This protects the case that a game result is contested, giving
            # sufficient time for player 1 to pick the correct result from the
            # ones learned.
            @current_player
            @guarantee
            def boards(self):
                for new in moves:
                    tx = TransactionTemplate()
                    tx.set_sequence(Weeks(1))
                    tx.add_output(self.amount, TicTacToeState(new, not player)(amount=self.amount, player_1=self.player_1, player_2=self.player_2))
                    yield tx
            # current player can accept next player's move at any time

            @current_player
            @next_player
            @guarantee
            def coop_boards(self):
                for new in moves:
                    tx = TransactionTemplate()
                    tx.add_output(self.amount, TicTacToeState(new, not player)(amount=self.amount, player_1=self.player_1, player_2=self.player_2))
                    yield tx

            # If no move is made in 2 weeks, any move can be made
            @guarantee
            def anymove(self):
                for new in moves:
                    tx = TransactionTemplate()
                    tx.set_sequence(Weeks(2))
                    tx.add_output(self.amount, TicTacToeState(new, not player)(amount=self.amount, player_1=self.player_1, player_2=self.player_2))
                    yield tx
        if unwinnable:
            @guarantee
            def tie(self):
                t = TransactionTemplate()
                amt = self.amount//2
                t.add_output(amt, PayToPubKey(amount=amt, key=self.player_1))
                t.add_output(amt, PayToPubKey(amount=amt, key=self.player_2))
                return t

    class W:
        Fields = TicTacToe.Fields

        @lru_cache
        def create_instance(self, **kwargs: Any) -> BindableContract[TicTacToe.Fields]:
            return TicTacToe(**kwargs)

        def __call__(self, amount, player_1, player_2):
            return self.create_instance(amount=amount, player_1=player_1, player_2=player_2)

    cache[board] = W()

    return cache[board]


TicTacToe = TicTacToeState(NewBoard(), False)
