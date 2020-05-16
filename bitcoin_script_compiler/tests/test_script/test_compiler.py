import unittest
from functools import reduce
from operator import and_, or_

import bitcoin_script_compiler
from bitcoin_script_compiler.clause import Wait, AbsoluteTimeSpec
from bitcoin_script_compiler.compiler import ClauseToDNF
import random


class TestCompiler(unittest.TestCase):
    def test_clause_to_cnf(self):
        clauses = [
            [Wait(AbsoluteTimeSpec.at_height(n)) for n in range(m * 100, (m + 1) * 100)]
            for m in range(100)
        ]
        # shuffle the clauses
        for clause in clauses:
            random.shuffle(clause)
        random.shuffle(clauses)
        anded = [reduce(and_, group[1:], group[0]) for group in clauses]
        orred = reduce(or_, anded[1:], anded[0])
        output = ClauseToDNF().compile_cnf(orred)
        self.assertSetEqual(
            frozenset(x.time.locktime for y in output for x in y),
            frozenset(range(0, 100 ** 2)),
            "does not preserve values",
        )
        self.assertSetEqual(
            frozenset(frozenset(x.time.locktime for x in y) for y in output),
            frozenset(frozenset(range(m * 100, (m + 1) * 100)) for m in range(100)),
            "does not preserves clauses",
        )

    def test_clause_to_cnf_random(self):
        A, B, C, D, E, F, G, H, I, J = [
            Wait(AbsoluteTimeSpec.at_height(n)) for n in range(10)
        ]
        inputs = ((((A | B) & C) | D | E | F) & G & H | I) | J
        # Checked by Wolfram Alpha...
        # (A ∧ C ∧ G ∧ H) ∨ (B ∧ C ∧ G ∧ H) ∨ (D ∧ G ∧ H) ∨ (E ∧ G ∧ H) ∨ (F ∧ G ∧ H) ∨ J ∨ K
        expected = [
            [A, C, G, H],
            [B, C, G, H],
            [D, G, H],
            [E, G, H],
            [F, G, H],
            [J],
            [I],
        ]
        output = ClauseToDNF().compile_cnf(inputs)
        to_set = lambda s: frozenset(frozenset(y.time.locktime for y in x) for x in s)
        self.assertSetEqual(to_set(output), to_set(expected), "Computes Correctly")


if __name__ == "__main__":
    unittest.main()
