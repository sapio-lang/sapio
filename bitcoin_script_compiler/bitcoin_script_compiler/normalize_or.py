from functools import singledispatchmethod
from typing import TYPE_CHECKING, Callable, Union

from .clause import (
    Wait,
    And,
    CheckTemplateVerify,
    Clause,
    Or,
    RevealPreImage,
    SignedBy,
    Unsatisfiable,
)


class NormalizationPass:
    """
    NormalizationPass takes an arbitrary clause and restructures it to bubble all of
    the OrClauses to the top-level.

    E.g., AndClause(OrClause(a,b), c) ==> OrClause(AndClause(a,c), AndClause(b,c))

    NormalizationPass tracks if it made any change on the past iteration, so that it can be called
    repeatedly until the algorithm has stabilized.

    NormalizationPass should be used in a loop until took_action is False, then the expression
    is fully normalized.
    """

    took_action: bool

    def __init__(self) -> None:
        self.took_action: bool = False

    def __call__(self, arg: Clause) -> Clause:
        if TYPE_CHECKING:
            # TODO: Required for singledispatchmethod to typecheck...
            assert callable(self.normalize)
        r: Clause = self.normalize(arg)
        return r

    # Normalize Bubbles up all the OR clauses into a DNF
    @singledispatchmethod
    def normalize(self, arg: Clause) -> Clause:
        raise NotImplementedError("Cannot Compile Arg", arg)

    @normalize.register
    def normalize_and(self, arg: And) -> Clause:
        if TYPE_CHECKING:
            # TODO: Required for singledispatchmethod to typecheck...
            assert callable(self.normalize)
        left: Clause = arg.left
        right: Clause = arg.right
        ret: Clause = arg
        if isinstance(left, Or) and isinstance(right, Or):
            self.took_action = True
            a0: Clause = self.normalize(left.left)
            a1: Clause = self.normalize(left.right)
            b0: Clause = self.normalize(right.left)
            b1: Clause = self.normalize(right.right)
            ret = (a0 & b0) | (a0 & b1) | (a1 & b0) | (a1 & b1)
        elif isinstance(right, And) and isinstance(left, Or):
            self.took_action = True
            _or, _and = self.normalize(left), self.normalize(right)
            ret = (_and & _or.left) | (_and & _or.right)
        elif isinstance(right, And) and isinstance(left, Or):
            self.took_action = True
            _or, _and = self.normalize(left), self.normalize(right)
            ret = (_and & _or.left) | (_and & _or.right)
        # Other Clause can be ignored...
        elif isinstance(left, And):
            ret = self.normalize(left) & right
        elif isinstance(left, Or):
            self.took_action = True
            a0, a1 = self.normalize(left.left), self.normalize(left.right)
            ret = (a0 & right) | (a1 & right)
        elif isinstance(right, And):
            ret = self.normalize(right) & left
        elif isinstance(right, Or):
            self.took_action = True
            b0, b1 = self.normalize(right.left), self.normalize(right.right)
            ret = (b0 & left) | (b1 & left)
        return ret

    @normalize.register
    def normalize_or(self, arg: Or) -> Clause:
        if TYPE_CHECKING:
            # TODO: Required for singledispatchmethod to typecheck...
            assert callable(self.normalize)
        # switching order guarantees that successive passes will
        # have a chance to unwind unsatisfiable clauses
        ret: Clause = self.normalize(arg.right) | self.normalize(arg.left)
        return ret

    @normalize.register(Unsatisfiable)
    @normalize.register(SignedBy)
    @normalize.register(RevealPreImage)
    @normalize.register(CheckTemplateVerify)
    @normalize.register(Wait)
    def normalize_unsat(
        self,
        arg: Union[
            (Unsatisfiable),
            (SignedBy),
            (RevealPreImage),
            (CheckTemplateVerify),
            (Wait),
        ],
    ) -> Clause:
        return arg
