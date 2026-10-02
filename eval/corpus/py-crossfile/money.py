def money_to_minor_units(amount, currency):
    return int(round(amount * 100))


def minor_unit_scale(currency):
    return 2
