# Minimal makefile for Sphinx documentation
#

# You can set these variables from the command line, and also
# from the environment for the first two.
SPHINXOPTS    ?=
SPHINXBUILD   ?= sphinx-build
SOURCEDIR     = source
BUILDDIR      = build

# Put it first so that "make" without argument is like "make help".
help:
	@$(SPHINXBUILD) -M help "$(SOURCEDIR)" "$(BUILDDIR)" $(SPHINXOPTS) $(O)


docs:
	sphinx-apidoc  --ext-autodoc -o source/sapio_compiler/ ../sapio_compiler/sapio_compiler
	sphinx-apidoc  --ext-autodoc -o source/sapio_zoo ../sapio_zoo/sapio_zoo
	sphinx-apidoc  --ext-autodoc -o source/sapio_stdlib ../sapio_stdlib/sapio_stdlib
	sphinx-apidoc  --ext-autodoc -o source/sapio_bitcoinlib ../sapio_bitcoinlib/sapio_bitcoinlib
	sphinx-apidoc  --ext-autodoc -o source/bitcoin_script_compiler ../bitcoin_script_compiler/bitcoin_script_compiler
	sphinx-apidoc  --ext-autodoc -o source/sapio_server ../sapio_server/sapio_server
	make html
cleanup:
	rm -r source/sapio_compiler/
	rm -r source/sapio_zoo
	rm -r source/sapio_stdlib
	rm -r source/sapio_bitcoinlib
	rm -r source/bitcoin_script_compiler
	rm -r source/sapio_server
	make clean


.PHONY: help Makefile

# Catch-all target: route all unknown targets to Sphinx using the new
# "make mode" option.  $(O) is meant as a shortcut for $(SPHINXOPTS).
%: Makefile
	@$(SPHINXBUILD) -M $@ "$(SOURCEDIR)" "$(BUILDDIR)" $(SPHINXOPTS) $(O)

