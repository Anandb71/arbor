# Provenance

`itsdangerous/{exc.py,encoding.py,signer.py}` are verbatim copies of real
repository code from [pallets/itsdangerous](https://github.com/pallets/itsdangerous)
(BSD-3-Clause, `LICENSE.txt` included alongside). They form a closed slice of
the package's intra-package imports — `signer` only reaches into `encoding`
and `exc` — so the fixture exercises real-world cross-file call resolution
against code the engine did not help write. `__init__.py` is an empty
placeholder so the files form a package; the real `__init__.py` re-exports
and is intentionally not used.
