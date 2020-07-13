from sapio_bitcoinlib.key import ECKey


def random_k():
    e = ECKey()
    e.generate()
    return e.get_pubkey()
