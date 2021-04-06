# Contract Actions

Contracts have a variety of different actions used at different times.


| name | function |
|------|---------|
| guard! | Create a clause using miniscript with access to the contract's values and context. |
| compile_if! | Determine if a `then!` or `finish!` should be compiled based on the contract's values and context |
| then! | Create a path or paths  that are guaranteed using CTV for a contract to be spent,  with optional `guard!`s and `compile_if!`s.|
| finish! | Create a suggested path or paths for a contract to be spent that are not guaranteed via CTV with mandatory `guard!`s and `compile_if!`s. Also accepts an update argument for generating transactions based on future data. |

This section will teach you then ins and outs of each.