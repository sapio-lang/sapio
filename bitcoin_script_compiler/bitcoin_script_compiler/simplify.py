import logging
from collections import defaultdict
from typing import List, Tuple, Union, cast

from .clause import (
    AbsoluteTimeSpec,
    Wait,
    CheckTemplateVerify,
    DNFClause,
    RelativeTimeSpec,
    Satisfied,
    Unsatisfiable,
)


class AfterClauseSimplification:
    """
    AfterClauseSimplification goes through a list of AfterClauses and reduces
    any CheckLockTimeVerify or CheckSequenceVerify lock times to at most two.
    It also sanity checks that the timeouts requested should either be relative
    or absolute but not both.

    It does not check that CTV is not used, which may externally conflict
    """

    PRUNE_MODE: bool = True
    """
    PRUNE_MODE can be configured to make incompatible timelocks an error, but
    the default behavior is to return an UnsatisfiableClause which results in a
    pruned DNF branch.
    """

    def simplify(
        self, clauses: List[Wait]
    ) -> Union[
        Unsatisfiable, Tuple[Union[Satisfied, Wait], Union[Satisfied, Wait]],
    ]:
        """
        Parameters
        ----------
        clauses
            list of all timing constraints in a DNF clause

        Returns
        -------
        Either UnsatisfiableClause or a pair of AfterClauses
        """
        log = logging.getLogger("compiler").getChild(self.__class__.__name__)
        # Filter out the relative and absolute clauses
        relative: List[RelativeTimeSpec] = []
        absolute: List[AbsoluteTimeSpec] = []
        for cl in clauses:
            if isinstance(cl.time, RelativeTimeSpec):
                relative.append(cl.time)
            elif isinstance(cl.time, AbsoluteTimeSpec):
                absolute.append(cl.time)
            else:
                raise ValueError("Unkown Type")

        # Filter the relative clauses into blocks and time
        relative_blocks: List[RelativeTimeSpec] = []
        relative_time: List[RelativeTimeSpec] = []
        for rel_ts in relative:
            if rel_ts.get_type() == "blocks":
                relative_blocks.append(rel_ts)
            elif rel_ts.get_type() == "time":
                relative_time.append(rel_ts)
            else:
                raise ValueError("Bad Literal")

        # Checks that there is only one type of time lock (otherwise
        # incompatible)
        if relative_time and relative_blocks:
            # Todo: Is this a true error? Or can we simply safely prune this branch...
            if self.PRUNE_MODE:
                log.warning("Incompatible Relative Time Locks! Pruning Branch")
                return Unsatisfiable()
            else:
                raise AssertionError("Incompatible Relative Time Locks in Branch")
        elif relative_blocks or relative_time:
            (_, rel_tl) = max(
                (rel_tl.time, rel_tl) for rel_tl in relative_blocks + relative_time
            )
            relative_return: Union[Wait, Satisfied] = Wait(rel_tl)
        else:
            relative_return = Satisfied()

        # filter the absolute clauses into blocks and time
        absolute_blocks: List[AbsoluteTimeSpec] = []
        absolute_time: List[AbsoluteTimeSpec] = []
        for abs_ts in absolute:
            if abs_ts.get_type() == "blocks":
                absolute_blocks.append(abs_ts)
            elif abs_ts.get_type() == "time":
                absolute_time.append(abs_ts)
            else:
                raise ValueError("Bad Literal")

        # Check that there is only one type of time lock (otherwise
        # incompatible)
        if absolute_time and absolute_blocks:
            # Todo: Is this a true error? Or can we simply safely prune this branch...
            if self.PRUNE_MODE:
                log.warning("Incompatible Absolute Time Locks! Pruning Branch")
                return Unsatisfiable()
            else:
                raise AssertionError("Incompatible Absolute Time Locks in Branch")
        elif absolute_time or absolute_blocks:
            (_, abs_tl) = max(
                (abs_tl.time, abs_tl) for abs_tl in absolute_blocks + absolute_time
            )
            absolute_return: Union[Wait, Satisfied] = Wait(abs_tl)
        else:
            absolute_return = Satisfied()
        return (relative_return, absolute_return)


class DNFSimplification:
    """
    DNFSimplification goes through a List[DNFClause] and performs simplifications
    based on the type of a clause.

    Currently this is limited to AfterClause reduction and common CTV elimination.

    DNFSimplification can also detect and mark a List[DNFClause] as
    unsatisfiable if certain conflicts show up

    Future work can eliminate repeated public-keys, use MuSig keys, check for repeated
    pre-images, and other simplifiers.

    Cross-branch common clause aggregation must happen at a separate layer.
    """

    PRUNE_MODE: bool = True
    """
    If detected conflicts should be ignored or raise an error
    """

    def simplify(self, all_clauses: List[DNFClause]) -> List[DNFClause]:
        clauses_to_return: List[DNFClause] = []
        log = logging.getLogger("compiler").getChild(self.__class__.__name__)
        clause_by_type = defaultdict(list)
        for cl in all_clauses:
            clause_by_type[type(cl)].append(cl)

        if Wait in clause_by_type:
            after_clauses = cast(List[Wait], clause_by_type.pop(Wait))
            val = AfterClauseSimplification().simplify(after_clauses)
            if isinstance(val, tuple):
                (a, b) = val
                if not isinstance(a, Satisfied):
                    clauses_to_return.append(a)
                if not isinstance(b, Satisfied):
                    clauses_to_return.append(b)
            else:
                return [Unsatisfiable()]
        if CheckTemplateVerify in clause_by_type:
            ctv_clauses = cast(
                List[CheckTemplateVerify], clause_by_type.pop(CheckTemplateVerify),
            )
            if len(ctv_clauses) <= 1:
                clauses_to_return.extend(list(ctv_clauses))
            else:
                first = ctv_clauses[0].hash
                if any(ctv.hash != first for ctv in ctv_clauses):
                    if self.PRUNE_MODE:
                        log.warning("Pruning Incompatible CheckTemplateVerify")
                        return [Unsatisfiable()]
                    else:
                        raise AssertionError("Incompatible CheckTemplateVerify Clause")
                else:
                    clauses_to_return.append(ctv_clauses[0])

        for (type_, clauses) in clause_by_type.items():
            clauses_to_return += clauses

        return clauses_to_return
