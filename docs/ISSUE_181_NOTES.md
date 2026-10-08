Draft — investigating #181 before writing a fix. My reproduction shows the resolver's SameFile branch is already ahead of cross-module candidates, and the extra caller of shadowed.py::process comes from hard/conditional.py::function\_local\_import, which is an expected edge per HARD\_GROUND\_TRUTH.md. Before proposing a code change I'd like confirmation on whether the fixture or grade.py check B is the thing to update.





