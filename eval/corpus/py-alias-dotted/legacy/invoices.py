from .money import money_to_minor_units as to_minor_units


def invoice_cents(lines):
    return sum(to_minor_units(line["amount"], "EUR") for line in lines)
