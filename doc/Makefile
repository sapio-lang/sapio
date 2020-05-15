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
	sphinx-apidoc -o source/sapio_compiler/ ../sapio_compiler/sapio_compiler
	sphinx-apidoc -o source/sapio_zoo ../sapio_zoo/sapio_zoo
	sphinx-apidoc -o source/sapio_stdlib ../sapio_stdlib/sapio_stdlib
	sphinx-apidoc -o source/bitcoinlib ../bitcoinlib/bitcoinlib
	sphinx-apidoc -o source/bitcoin_script_compiler ../bitcoin_script_compiler/bitcoin_script_compiler
	make html


.PHONY: help Makefile

# Catch-all target: route all unknown targets to Sphinx using the new
# "make mode" option.  $(O) is meant as a shortcut for $(SPHINXOPTS).
%: Makefile
	@$(SPHINXBUILD) -M $@ "$(SOURCEDIR)" "$(BUILDDIR)" $(SPHINXOPTS) $(O)

