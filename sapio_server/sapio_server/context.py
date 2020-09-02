from sapio_compiler import Contract
from typing import Dict, Optional


class Context:
    compilation_cache: Dict[str, Contract]

    def __init__(self):
        self.compilation_cache = {}

    def cache(self, k: str, b: Contract ):
        self.compilation_cache[k] = b

    def uncache(self, k: str) -> Optional[Contract]:
        try:
            return self.compilation_cache[k]
        except KeyError:
            return None
