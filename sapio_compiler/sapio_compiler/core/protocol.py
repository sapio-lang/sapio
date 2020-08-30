from __future__ import annotations

import copy
import inspect
import typing
from typing import Any, Dict, List, Type, TYPE_CHECKING, Generic


from sapio_bitcoinlib import miniscript
from sapio_bitcoinlib.messages import COutPoint, CTransaction, CTxInWitness, CTxWitness
from bitcoin_script_compiler import (
    CheckTemplateVerify,
    Clause,
    CTVHash,
    Satisfied,
    Unsatisfiable,
    WitnessManager,
    Threshold,
)
from sapio_bitcoinlib.static_types import Amount, Hash, Sats
from sapio_bitcoinlib.script import CScript
from typing import (
    Protocol,
    TypedDict,
    ClassVar,
    Callable,
    Tuple,
    Optional,
    Iterator,
    Union,
    Generator,
    Iterable,
    TypeVar,
    Final,
    overload,
)
from .amountrange import AmountRange
from functools import wraps
import itertools
import functools
from abc import abstractmethod
from dataclasses import dataclass, asdict

import rust_miniscript

if TYPE_CHECKING:
    import sapio_compiler.core.txtemplate as txtmpl
from .txtemplate import TransactionTemplate


class Trait(Protocol):
    pass


class FuncIndex:
    def __init__(self, i: int) -> None:
        self.i = i


class ThenFuncIndex(FuncIndex):
    pass


class FinishFuncIndex(FuncIndex):
    pass


class FinishOrFuncIndex(FuncIndex):
    pass


Props = TypeVar("Props")
IndexType = TypeVar("IndexType", bound=FuncIndex)

Finisher = Callable[[Props], Clause]
TxRetType = Union["txtmpl.TransactionTemplate", Iterator["txtmpl.TransactionTemplate"]]
ThenF = Callable[[Props], TxRetType]


class ContractProtocol(Protocol[Props]):
    Props: ClassVar[Type[Props]]
    # Class Variables
    then_funcs: ClassVar[List[Tuple[ThenF[Props], List[Finisher[Props]]]]]
    finish_or_funcs: ClassVar[List[Tuple[ThenF[Props], List[Finisher[Props]]]]]
    finish_funcs: ClassVar[List[Tuple[ThenF[Props], List[Finisher[Props]]]]]
    assert_funcs: ClassVar[List[Callable[[Props], bool]]]

    """
    A PayAddress function is a special type which stubs out
    the contract as being just the amount/address combo returned

    If a PayAddress decorator is used, no other functions may be present,
    except assertions.

    This is useful to avoid creating an intermediate txn to patch in an address.

    Mostly for use by Library Writers, but useful to expose in case one
    wants to add validation logic for passed in addresses (e.g., consulting
    a local node/wallet to check if the key is known).
    """
    override: Optional[Callable[[Props], Tuple[AmountRange, str]]]


    """
    let declares a requirement variable, but does not
    enforce it.

    Layered requirement allows one to create custom requirement decorators
    which can wrap UnlockFunctions or PathFunctions.

    This allows one to build up the set of conditions by which a particular
    branch may be spent.

    Examples
    --------
    >>> class A(Contract):
    ...     class Fields:
    ...         pk: PubKey
    ...     @require
    ...     def signed(self):
    ...         return SignatureCheckClause(self.pk)
    ...     @signed
    ...     @unlock
    ...     def spend(self):
    ...         return Satisfied()
    """

    @classmethod
    def let(cls, c: Finisher[Props]) -> Union[Callable[[IndexType], IndexType], Callable[[ Callable[[IndexType], [IndexType]]], Callable[[IndexType], IndexType]]]:
        @overload
        def wrapper(x: None) -> None:
            raise NotImplementedError()
        @overload
        def wrapper(x: IndexType) -> IndexType:
            raise NotImplementedError()
        @overload
        def wrapper(f: Callable[[IndexType], IndexType]) -> Callable[[IndexType], IndexType]:
            raise NotImplementedError()

        @wraps(c)
        def wrapper(arg: Any) -> Any:
            if isinstance(arg, FuncIndex):
                x: FuncIndex = arg
                if isinstance(x, ThenFuncIndex):
                    cls.then_funcs[x.i][1].append(c)
                elif isinstance(x, FinishOrFuncIndex):
                    cls.finish_or_funcs[x.i][1].append(c)
                elif isinstance(x, FinishFuncIndex):
                    cls.finish_funcs[x.i][1].append(c)
                else:
                    raise ValueError("Invalid FuncIndex Instance")
                return x
            if callable(arg):
                f: Callable[[IndexType], IndexType] = arg
                def inner_wrapper(y: IndexType) -> IndexType:
                    return wrapper(f(y))
                return inner_wrapper
            else:
                raise ValueError("Cannot Use Wrapper")
        return wrapper


    @classmethod
    def then(cls, c: ThenF[Props]) -> ThenFuncIndex:

        """
        A path function is a type of function which must return either a
        TransactionTemplate or an Iterator[TransactionTemplate].

        There are two fundamental ways of constructing a PathFunction, either for a
        guarantee-d path (using PathFunction.guarantee) or a unlock_but_suggeste-d
        path (using PathFunction.unlock_but_suggest). The difference is that
        guarantee instructs the compiler to use CheckTemplateVerify to ensure the
        outcomes whereas unlock_but_suggest does not.
        """
        cls.then_funcs.append((c, []))
        return ThenFuncIndex(len(cls.then_funcs) - 1)

    @classmethod
    def finish_or(cls, c: ThenF[Props]) -> FinishOrFuncIndex:
        """
        A path function is a type of function which must return either a
        TransactionTemplate or an Iterator[TransactionTemplate].

        There are two fundamental ways of constructing a PathFunction, either for a
        guarantee-d path (using PathFunction.guarantee) or a unlock_but_suggeste-d
        path (using PathFunction.unlock_but_suggest). The difference is that
        guarantee instructs the compiler to use CheckTemplateVerify to ensure the
        outcomes whereas unlock_but_suggest does not.

        This is useful for HTLC based protocols.
        """
        cls.finish_or_funcs.append((c, []))
        return FinishOrFuncIndex(len(cls.finish_or_funcs) - 1)

    @classmethod
    def finish(cls, c: Finisher[Props]) -> FinishFuncIndex:
        """
        An UnlockFunction expresses a keypath spending. There are no further
        restrictions on how a coin may be spent.
        """

        def mock(x: Props) -> Iterator[TransactionTemplate]:
            return iter([])

        mock.__name__ = c.__name__
        cls.finish_funcs.append((mock, [c]))
        return FinishFuncIndex(len(cls.finish_funcs) - 1)

    @classmethod
    def threshold(
        cls, n: int, l: List[Callable[[Props], Clause]]
    ) -> Callable[[IndexType], IndexType]:
        """
        threshold takes combinations of length N of conditions from the provided
        list and allows any such group to satisfy.
        """
        if not len(l) >= n:
            raise ValueError("Expected to get more conditions in threshold")
        if not n > 0:
            raise ValueError("Threshold int must be positive")

        @cls.let
        def inner(c: Props) -> Clause:
            clauses: List[Clause] = [let_func(c) for let_func in l]
            return Threshold(n, clauses)

        return inner

    @classmethod
    def require(cls, f: Callable[[Props], bool]) -> None:
        """
        A CheckFunction decorator should return a function that either raises its
        own exception or returns True/False.

        Raising your own exception is preferable because it can help users
        debug their own contracts more readily.
        """
        cls.assert_funcs.append(f)

    def bind(
        self, out_in: COutPoint
    ) -> Tuple[List[CTransaction], List[Dict[str, Any]]]:
        """
        Attaches a BindableContract to a specific COutPoint and generates all
        the child transactions along with metadata entries
        """
        # todo: Note that if a contract has any secret state, it may be a hack
        # attempt to bind it to an output with insufficient funds

        txns = []
        metadata_out = []
        queue: List[Tuple[COutPoint, ContractProtocol[Any]]] = [(out_in, self)]

        while queue:
            out, this = queue.pop()
            metadata = getattr(this.data, "metadata", None)
            color = getattr(metadata, "color", "green")
            contract_name = getattr(metadata, "label", "unknown")
            program = this.witness_manager.program
            is_ctv_before = len(this.then_funcs)
            for (i, (func, _)) in enumerate(
                itertools.chain(
                    this.then_funcs, this.finish_or_funcs # , this.finish_funcs #
                )
            ):
                is_ctv = i < is_ctv_before
                templates = this.txn_abi[func.__name__][1]
                for txn_template in templates:
                    ctv_hash = txn_template.get_ctv_hash()

                    # This uniquely binds things with a CTV hash to the
                    # appropriate witnesses. Also binds things with None to all
                    # possible witnesses that do not have a ctv
                    ctv_sat = (miniscript.SatType.TXTEMPLATE, ctv_hash)
                    candidates = [
                        wit for wit in this.witness_manager.ms.sat if (ctv_sat in wit if is_ctv else all(w[0] != miniscript.SatType.TXTEMPLATE for w in wit))
                    ]
                    # There should always be a candidate otherwise we shouldn't
                    # have a txn
                    if not candidates:
                        raise AssertionError("There must always be a candidate")

                    # todo: find correct witness?
                    tx_label = contract_name + ":" + txn_template.label
                    tx = txn_template.bind_tx(out)
                    tx.wit = CTxWitness()
                    tx.wit.vtxinwit.append(CTxInWitness())
                    # Create all possible candidates
                    for wit in candidates:
                        t = copy.deepcopy(tx)
                        t.wit.vtxinwit[0].scriptWitness.stack = wit + [
                            (miniscript.SatType.DATA, program)
                        ]
                        txns.append(t)
                        utxo_metadata = [
                            md.to_json() for md in txn_template.outputs_metadata
                        ]
                        metadata_out.append(
                            {
                                "color": color,
                                "label": tx_label,
                                "utxo_metadata": utxo_metadata,
                            }
                        )
                    txid = int(tx.hash or tx.rehash(), 16)
                    for (i, (_, contract)) in enumerate(txn_template.outputs):
                        # TODO: CHeck this is correct type into COutpoint
                        queue.append((COutPoint(txid, i), contract))

        return txns, metadata_out
    @classmethod
    def create(cls, **kwargs: Any) -> ContractProtocol[Props]:
        """Convenience -- type inference may not work!"""
        return cls(cls.Props(**kwargs))
        
def reduce_and(c: Clause, c2: Clause) -> Clause:
    return c & c2
def mapargs(*args:Any):
    def apply(f: Callable[[X], Y]) -> Y:
        return f(*args)
    return apply
class ContractBase(Generic[Props]):
    # Instance Variables
    data: Props
    txn_abi: Dict[str, Tuple[ThenF[Props], List["txtmpl.TransactionTemplate"]]]
    conditions_abi: Dict[str, Tuple[ThenF[Props], Clause]]
    witness_manager: WitnessManager
    amount_range: AmountRange
    # kwargs does not support typeddict
    def __init__(self, props: Props) -> None:
        self.data = props
        amount_range = AmountRange()
        # Check all assertions. Assertions should not return anything.
        if not all(assert_func(self.data) for assert_func in self.assert_funcs):
            raise AssertionError(
                f"CheckFunction for {self.name} did not throw any error, but returned False"
            )
        txn_abi: Dict[str, Tuple[ThenF[Props], List[TransactionTemplate]]] = {}
        conditions_abi = {}
        if self.override is not None:
            if self.finish_funcs or self.finish_or_funcs or self.then_funcs:
                raise ValueError("Overriden Contract has other branches")
            amt_rng, addr = self.override.__func__(self.data)
            amount_range = amt_rng
            witness_manager = WitnessManager(miniscript.Node())
            witness_manager.override_program = addr
        else:
            # Get the value from all paths.
            # Paths return a TransactionTemplate object, or list, or iterable.
            paths: Clause = Unsatisfiable()
            use_ctv_before = len(self.then_funcs)
            add_tx_before = len(self.then_funcs) + len(self.finish_or_funcs)
            for (i, (func, conditions)) in enumerate(
                itertools.chain(
                    self.then_funcs, self.finish_or_funcs, self.finish_funcs
                )
            ):
                conditions = functools.reduce(reduce_and, map(mapargs(self.data), conditions), Satisfied())
                conditions_abi[func.__name__] = ( func,conditions)
                all_ctvs: Clause = Unsatisfiable()
                if i < add_tx_before:
                    txn_abi[func.__name__] = (func, [])
                    # Run the path function
                    ret: TxRetType = func(self.data)
                    # Coerce to an iterator
                    transaction_templates: Iterator[TransactionTemplate]
                    if isinstance(ret, TransactionTemplate):
                        # Wrap value for uniform handling below
                        transaction_templates = iter([ret])
                    elif isinstance(ret, (Generator, Iterable)):
                        transaction_templates = ret
                    else:
                        raise ValueError("Invalid Return Type", ret)
                    for template in transaction_templates:
                        template.finalize()
                        template.label = func.__name__
                        amount = template.total_amount()
                        amount_range.update_range(amount)
                        # not all transactions are guaranteed
                        if i < use_ctv_before:
                            # ctv_hash is an identifier and a txid equivalent
                            ctv_hash = template.get_ctv_hash()
                            # TODO: If we OR all the CTV hashes together
                            # and then and at the top with the unlock clause,
                            # it could help with later code generation sharing the
                            ctv = CheckTemplateVerify(Hash(ctv_hash))
                            all_ctvs |= ctv
                        txn_abi[func.__name__][1].append(template)
                if i >= use_ctv_before:
                    paths |= conditions_abi[func.__name__][1]
                else:
                    paths |= (conditions_abi[func.__name__][1] & all_ctvs)
            if isinstance(paths, Unsatisfiable):
                raise AssertionError("Must Have at least one spending condition")
            desc = paths.to_miniscript()
            script = CScript(rust_miniscript.compile_policy(bytes(desc, "utf-8")))
            ms = miniscript.Node.from_script(script)
            witness_manager = WitnessManager(ms)

        # Set Instance Variables at end (Immutable)
        self.txn_abi = txn_abi
        self.conditions_abi = conditions_abi
        self.witness_manager = witness_manager
        self.amount_range = amount_range
        getattr(self.data, "__finalize__", lambda s: None)(self)
