import os

from openai import OpenAI

from .factory import make_client
from .settings import MODEL_NAME

client = OpenAI()
ENV_MODEL = os.getenv("OPENAI_MODEL", "gpt-3.5-turbo")


def from_env(prompt):
    return client.chat.completions.create(model=ENV_MODEL, messages=[{"role": "user", "content": prompt}])


def from_fstring(prompt, version):
    return client.chat.completions.create(model=f"gpt-{version}", messages=[{"role": "user", "content": prompt}])


def from_settings(prompt):
    return client.chat.completions.create(model=MODEL_NAME, messages=[{"role": "user", "content": prompt}])


def from_kwargs(**params):
    return client.chat.completions.create(**params)


def dispatch(method, prompt):
    return getattr(client.chat.completions, method)(model="gpt-4", messages=[{"role": "user", "content": prompt}])


def factory_made(prompt):
    llm = make_client()
    return llm.chat.completions.create(model="gpt-3.5-turbo", messages=[{"role": "user", "content": prompt}])


def injected(llm, prompt):
    return llm.chat.completions.create(model="gpt-4o-mini", messages=[{"role": "user", "content": prompt}])


create = client.chat.completions.create
