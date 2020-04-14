from setuptools import setup, find_packages


with open('README.rst') as f:
    readme = f.read()

with open('LICENSE') as f:
    license = f.read()

setup(
    name='sapio',
    version='0.1.0',
    description='Bitcoin Transaction Programming Language',
    long_description=readme,
    author='Jeremy Rubin',
    author_email='j@rubin.io',
    url='https://github.com/jeremyrubin/sapio',
    license=license,
    packages=find_packages(exclude=('tests', 'docs')))
