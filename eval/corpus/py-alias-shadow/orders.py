from money import money_to_minor_units as to_minor


def total_cents(items):
    return sum(to_minor(item["price"], "USD") for item in items)
