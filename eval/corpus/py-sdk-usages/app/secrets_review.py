"""The five credential leaks from the review of #276. Nothing here may appear
in the output: every value is credential-shaped or flows through a
credential-named binding."""
from openai import OpenAI

client = OpenAI()
OPENAI_API_KEY = "sk-proj-EVALSECRETONE000000000000"


def from_constant():
    return client.chat.completions.create(model=OPENAI_API_KEY, messages=[])


def from_literal():
    return client.chat.completions.create(model="sk-ant-api03-EVALSECRETTWO0000000", messages=[])


def from_default(token="sk-EVALSECRETTHREE00000000000"):
    return client.chat.completions.create(model=token, messages=[])


class Holder:
    def __init__(self):
        self.secret = "sk-EVALSECRETFOUR000000000000"

    def go(self):
        return client.chat.completions.create(model=self.secret, messages=[])


def dict_key():
    return client.chat.completions.create(
        model="gpt-4o", messages=[], extra_body={"sk-proj-EVALSECRETFIVE00000000": {"model": "gpt-4o"}}
    )
