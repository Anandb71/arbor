"""Each use resolved where it runs: comprehension targets, a function called
between rebindings, a class body, and a client rebound in another branch of
the same loop (Track C h042)."""
import anthropic
from openai import OpenAI

client = OpenAI()
model = "gpt-4o"

listed = [client.chat.completions.create(model=model, messages=[]) for model in ["gpt-3.5-turbo", "o1-mini"]]

MODEL = "o1-mini"


def ask(q):
    return client.chat.completions.create(model=MODEL, messages=[q])


print(ask("hi"))
MODEL = "gpt-4o"

CL7 = OpenAI()


class Bot:
    c = CL7

    def go(self):
        return self.c.chat.completions.create(model="gpt-4", messages=[])


CL7 = OpenAI(base_url="https://gateway.example/v1")


def run(providers):
    for provider in providers:
        if provider == "openai":
            llm = OpenAI()
            llm.chat.completions.create(model="gpt-4o", messages=[])
        else:
            llm = anthropic.Anthropic()
            llm.messages.create(model="claude-3-opus-20240229", max_tokens=1, messages=[])
    return llm
