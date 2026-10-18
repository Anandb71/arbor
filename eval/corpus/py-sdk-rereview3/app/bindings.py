"""Bindings the third review of #276 found uncounted: `nonlocal`, a `match`
capture, an `except` target and a write through `globals()`."""
import os

from openai import AzureOpenAI, OpenAI

client = OpenAI()


def switch_client():
    c = OpenAI()

    def to_azure():
        nonlocal c
        c = AzureOpenAI()

    to_azure()
    return c.chat.completions.create(model="gpt-4o", messages=[])


def switch_model():
    model = "gpt-4o"

    def pick():
        nonlocal model
        model = "o1-mini"

    pick()
    return client.chat.completions.create(model=model, messages=[])


MODEL5 = "gpt-4o"
match os.getenv("M"):
    case str() as MODEL5:
        pass
client.chat.completions.create(model=MODEL5, messages=[])

MODEL11 = "gpt-4o"
try:
    pass
except Exception as MODEL11:
    pass
client.chat.completions.create(model=MODEL11, messages=[])

MODEL7 = "gpt-4o"
globals()["MODEL7"] = "o3-mini"
client.chat.completions.create(model=MODEL7, messages=[])


def tune():
    globals()["MODEL8"] = "o1-mini"


MODEL8 = "gpt-4o"
client.chat.completions.create(model=MODEL8, messages=[])
