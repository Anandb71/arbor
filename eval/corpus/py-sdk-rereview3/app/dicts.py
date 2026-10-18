"""Dicts aliased through a chained assignment, a class attribute, `self`
and `globals()`: changed after they are written, so never read."""
from openai import OpenAI

client = OpenAI()

CFG1 = {"model": "gpt-4o"}


class K1:
    cfg = CFG1


K1.cfg["model"] = "o1-mini"
client.chat.completions.create(**CFG1, messages=[])

CFG2 = {"model": "gpt-4o"}
x = y = CFG2
x["model"] = "o1-mini"
client.chat.completions.create(**CFG2, messages=[])

CFG3 = {"model": "gpt-4o"}


class K3:
    cfg = CFG3

    def tune(self):
        self.cfg["model"] = "o3-mini"


client.chat.completions.create(model=CFG3["model"], messages=[])

CFG7 = {"model": "gpt-4o"}
globals()["CFG7"]["model"] = "o1-mini"
client.chat.completions.create(**CFG7, messages=[])

READ_ONLY = {"model": "gpt-4.1"}
alias = other = READ_ONLY
print(alias["model"])
client.chat.completions.create(**READ_ONLY, messages=[])
