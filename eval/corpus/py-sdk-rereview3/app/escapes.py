"""Functions used as values: their callers cannot all be seen, so a default
keeps a gap. A public function's client default keeps one too."""
import functools
from concurrent.futures import ThreadPoolExecutor

from openai import OpenAI

client = OpenAI()
gw = OpenAI(base_url="https://gateway.example/v1")


def submitted(q, model="gpt-4o"):
    return client.chat.completions.create(model=model, messages=[q])


with ThreadPoolExecutor() as pool:
    pool.submit(submitted, "hi", "o1-mini")


def partial_model(q, model="gpt-4o"):
    return client.chat.completions.create(model=model, messages=[q])


fast = functools.partial(partial_model, model="o1-mini")


def mapped(q, model="gpt-4o"):
    return client.chat.completions.create(model=model, messages=[q])


list(map(mapped, ["a"], ["o1-mini"]))


def task(f):
    return f


@task
def queued(q, model="gpt-4o"):
    return client.chat.completions.create(model=model, messages=[q])


queued.delay("hi", model="o1-mini")


def partial_client(q, c=client):
    return c.chat.completions.create(model="gpt-4", messages=[q])


via_gw = functools.partial(partial_client, c=gw)


def public_client(q, c=client):
    return c.chat.completions.create(model="gpt-4", messages=[q])


def _private_client(q, c=client):
    return c.chat.completions.create(model="gpt-4", messages=[q])


_private_client("x")
