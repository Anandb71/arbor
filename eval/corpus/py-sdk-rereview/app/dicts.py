"""Dict literals changed after they are written, through a chain, an alias, a
helper or an attribute: never read as written."""
from openai import OpenAI

client = OpenAI()

CFG = {"openai": {"model": "gpt-4o"}}
CFG["openai"]["model"] = "o1-mini"
client.chat.completions.create(**CFG["openai"], messages=[])

P3 = {"model": "gpt-4o"}
alias = P3
alias["model"] = "gpt-3.5-turbo"
client.chat.completions.create(**P3, messages=[])

P4 = {"model": "gpt-4o"}


def tweak(d):
    d["model"] = "o4-mini"


tweak(P4)
client.chat.completions.create(**P4, messages=[])


class Holder:
    pass


P5 = {"model": "gpt-4o"}
Holder.d = P5
Holder.d["model"] = "gpt-4-0613"
client.chat.completions.create(model=P5["model"], messages=[])

READ_ONLY = {"model": "gpt-4.1"}
print(len(READ_ONLY))
client.chat.completions.create(**READ_ONLY, messages=[])
