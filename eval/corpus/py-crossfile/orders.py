from money import money_to_minor_units


def total_cents(items):
    return sum(money_to_minor_units(item["price"], "USD") for item in items)


def checkout_total(order, discount):
    cents = total_cents(order["items"])
    return apply_discount(cents, discount)


def apply_discount(cents, discount):
    return cents - int(cents * discount / 100)
