"""Constructor calls run `__init__`: a default stored on `self` and a
client default are weighed against them."""
from openai import OpenAI

client = OpenAI()


class Agent:
    def __init__(self, model="gpt-4o"):
        self.model = model

    def run(self):
        return client.chat.completions.create(model=self.model, messages=[])


Agent(model="o1-mini").run()


class Service:
    def __init__(self, c=client):
        self.c = c

    def go(self):
        return self.c.chat.completions.create(model="gpt-4", messages=[])


Service(OpenAI(base_url="https://gateway.example/v1")).go()


class Quiet:
    def __init__(self, model="gpt-4o-mini"):
        self.model = model

    def run(self):
        return client.chat.completions.create(model=self.model, messages=[])


Quiet().run()
