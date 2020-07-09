test:
	cd sapio_bitcoinlib && $(MAKE) test
	cd bitcoin_script_compiler && $(MAKE) test
	cd sapio_compiler && $(MAKE) test
	cd sapio_stdlib && $(MAKE) test
	cd sapio_zoo && $(MAKE) test
	cd sapio_server && $(MAKE) test
