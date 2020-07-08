from setuptools import setup, find_packages


with open("README.md") as f:
    readme = f.read()

with open("LICENSE") as f:
    license = f.read()

setup(
    name="rust_miniscript_bindings",
    version="0.1.0",
    description="Rust Miniscript Python Bindings",
    long_description=readme,
    author="Jeremy Rubin",
    author_email="j@rubin.io",
    url="https://github.com/sapio-lang/sapio",
    license=license,
    packages=find_packages(exclude=("tests", "docs")),
)
