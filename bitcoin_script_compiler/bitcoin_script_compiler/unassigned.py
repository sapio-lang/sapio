from .clause import SignatureCheckClause, PreImageCheckClause


class Variable:
    """
    Variable is a base class for different types of unfilled data that may come up,
    such as signatures or pre-images.

    It's not a Union so that type-based dispatch works appropriately.

    Variables are aware of the type of data that is meant to fill them (e.g.,
    signature or pre-image), which aids in transaction finalization.
    """


class SignatureVar(Variable):
    """
    A missing Signature
    """

    def __init__(self, pk: SignatureCheckClause) -> None:
        self.pk = pk.pubkey


class PreImageVar(Variable):
    """
    A missing PreImage
    """

    def __init__(self, image: PreImageCheckClause) -> None:
        self.image = image.image
