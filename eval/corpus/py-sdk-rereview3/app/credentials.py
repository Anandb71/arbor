"""Credential shapes through the new candidate lists: loop items, a dict key
that is not an identifier, a conditional, `setattr` and a key that is itself
credential-shaped. Nothing here may reach the output."""
import os

from openai import OpenAI

client = OpenAI()

for item in ["sk-proj-EVALRR3FAKEFAKEFAKEFAKE0000", "gpt-4o"]:
    client.chat.completions.create(model=item, messages=[])

ODD = {
    "fast-tier": "sk-proj-EVALRR3FAKEFAKEFAKEFAKE0001",
    "api-key": "EVALRR3-SECRET",
    "sk-proj-EVALRR3FAKEFAKEFAKEFAKE0002": "gpt-4o",
}
client.chat.completions.create(model=ODD[os.getenv("T")], messages=[])

client.chat.completions.create(
    model="sk-proj-EVALRR3FAKEFAKEFAKEFAKE0003" if os.getenv("Q") else "gpt-4o",
    messages=[],
)


class Holder:
    model = "gpt-4o"

    def go(self):
        return client.chat.completions.create(model=self.model, messages=[])


h = Holder()
setattr(h, "model", "sk-proj-EVALRR3FAKEFAKEFAKEFAKE0004")
