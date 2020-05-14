from setuptools import setup, find_packages


with open("README.md") as f:
    readme = f.read()

with open("LICENSE") as f:
    license = f.read()

setup(
    name="sapio_zoo",
    version="0.1.0",
    description="Bitcoin Transaction Programming Language",
    long_description=readme,
    author="Jeremy Rubin",
    author_email="j@rubin.io",
    url="https://github.com/jeremyrubin/sapio",
    license=license,
    packages=find_packages(exclude=("tests", "docs")),
    install_requires=[
        "mypy>=0.770",
        "mypy-extensions>=0.4.3",
        "numpy>=1.18.0",
        "typed-ast>=1.4.0",
        "typing-extensions>=3.7.4.2",
    ],
    extras_require={"server": ["tornado>=6.0"]},
)
