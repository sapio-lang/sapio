Sapio Bitcoin TX PL Docs
=================================
Sapio is a framework for creating composable multi-transaction Bitcoin Smart Contracts.

Why is Sapio Different?
-----------------------
Sapio helps you build payment protocol specifiers that oblivious third parties
can participate in being none the wiser.

For example, with Sapio you can generate an address that represents a lightning
channel between you and friend and give that address to a third party service
like an exchange and have them create the channel without requiring any
signature interaction from you or your friend, zero trusted parties, and an
inability to differentiate your address from any other.

That's the tip of the iceberg of what Sapio lets you accomplish.


.. toctree::
    :maxdepth: 1

    bitcoin_script_compiler/modules.rst
    sapio_bitcoinlib/modules.rst
    sapio_compiler/modules.rst
    sapio_stdlib/modules.rst
    sapio_zoo/modules.rst
    sapio_server/modules.rst



Indices and tables
==================

* :ref:`genindex`
* :ref:`modindex`
* :ref:`search`
