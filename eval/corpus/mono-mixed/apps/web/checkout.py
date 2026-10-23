import sys
sys.path.insert(0, "../../packages/core")
from money import money_to_minor_units


def total(items):
    return sum(money_to_minor_units(i, "USD") for i in items)
