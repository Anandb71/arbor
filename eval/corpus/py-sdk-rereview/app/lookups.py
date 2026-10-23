"""Models picked at run time: a dict looked up with a run-time key, a name
assigned in two branches (also through a parameter default), a request with
no model, and env/config fallbacks nested in a request."""
import os

from openai import OpenAI

client = OpenAI()
MODELS = {"fast": "gpt-4o-mini", "smart": "o1-mini", "api_key": "EVALRR2-DICTSECRET"}

client.chat.completions.create(model=MODELS[os.getenv("TIER")], messages=[])

if os.getenv("FAST"):
    PICK = "gpt-3.5-turbo-0125"
else:
    PICK = "gpt-4-0613"


def ask(q, model=PICK):
    return client.chat.completions.create(model=model, messages=[q])


client.chat.completions.create(messages=[])
client.chat.completions.create(
    model="gpt-4o", messages=[], extra_body={"fallbacks": [{"model": os.getenv("FALLBACK", "gpt-4-0314")}]}
)
