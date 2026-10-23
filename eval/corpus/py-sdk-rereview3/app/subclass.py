"""`self.X` may be a subclass's: an overriding class attribute and a
swapped client."""
from openai import OpenAI


class Base:
    MODEL = "gpt-4o"
    client = OpenAI()

    def go(self):
        return self.client.chat.completions.create(model=self.MODEL, messages=[])


class Fast(Base):
    MODEL = "o1-mini"
    client = OpenAI(base_url="https://gateway.example/v1")


Fast().go()


class Plain:
    MODEL = "gpt-4.1"
    client = OpenAI()

    def go(self):
        return self.client.chat.completions.create(model=self.MODEL, messages=[])
