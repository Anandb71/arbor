import os

from evals.sampler import Sampler


def main():
    bench = Sampler(model=os.getenv("BENCH_MODEL", "o3-mini"))
    return bench
