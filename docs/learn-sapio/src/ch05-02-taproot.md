# Taproot


Sapio contract logic can become very large in size, so Sapio benefits from
being able to split up and merkelize the logic into smaller satisfiable
chunks. This makes it much more economical and easy to use Sapio however you like.

Generally speaking, a Sapio programmer need not think about this too much, it will be set up
automatically under the hood. However, at writing, limited optimizing of Taproot trees is done,
so a wise programmer would want to express their program in such a way to not allow Taproot leafs to be larger than need be.

