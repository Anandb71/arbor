"""Shapes the first real-world evaluation missed or mis-routed: constant
parameter defaults, env/config fallbacks, same-file dict literals, a client
handed to a helper, `with_options(base_url=...)`, a rebinding at module level
and calls that used to drop their model silently."""
import os

from decouple import config
from openai import OpenAI

GPT_MODEL = "gpt-3.5-turbo"
PARAMS = {"model": "gpt-4o-mini", "temperature": 0}
MODELS = {"fast": "gpt-4o-mini", "smart": "o3"}

client = OpenAI()


def ask(question, model: str = GPT_MODEL):
    return client.chat.completions.create(model=model, messages=[{"role": "user", "content": question}])


def from_env(question):
    return client.chat.completions.create(model=os.getenv("CHAT_MODEL", "gpt-4"), messages=[question])


def from_config(question):
    return client.chat.completions.create(model=config("CHAT_MODEL", default="gpt-4-turbo"), messages=[question])


def from_params(question):
    return client.chat.completions.create(**PARAMS, messages=[question])


def from_table(question):
    return client.chat.completions.create(model=MODELS["smart"], messages=[question])


def send(llm, question):
    return llm.chat.completions.create(model="gpt-4.1", messages=[question])


def run(question):
    return send(client, question)


def routed(question):
    proxy = client.with_options(base_url="http://localhost:4000")
    return proxy.chat.completions.create(model="gpt-4o", messages=[question])


def draw(prompt):
    return client.images.generate(model=os.environ["IMAGE_MODEL"], prompt=prompt)


handler = lambda client, q: client.chat.completions.create(model="gpt-4o", messages=[q])
