"""Parameters: defaults that callers override, client defaults weighed against
callers, a dict.get default resolved where the call runs, and a client
passed by the in-file callers of a public function."""
from openai import OpenAI

client = OpenAI()
gw = OpenAI(base_url="https://gateway.example/v1")
DEFAULTS = {"temperature": 0}
M = "gpt-3.5-turbo-instruct"


def ask(q, model="gpt-4o"):
    return client.chat.completions.create(model=model, messages=[q])


ask("hi", model="gpt-3.5-turbo")
ask("hi", "o1-mini")


def ask_quiet(q, model="gpt-4o-mini"):
    return client.chat.completions.create(model=model, messages=[q])


ask_quiet("hi")


def local_default():
    M = "gpt-4o"
    return client.chat.completions.create(model=DEFAULTS.get("model", M), messages=[])


def via_default(q, c=client):
    return c.chat.completions.create(model="gpt-4", messages=[q])


via_default("x", c=gw)


def _private_default(q, c=client):
    return c.chat.completions.create(model="gpt-4", messages=[q])


_private_default("x", c=gw)

quick = lambda q, model="gpt-4o": client.chat.completions.create(model=model, messages=[q])
quick("x", model="gpt-3.5-turbo")


def send(llm, q):
    return llm.chat.completions.create(model="gpt-4.1", messages=[q])


def _send(llm, q):
    return llm.chat.completions.create(model="gpt-4.1", messages=[q])


def run(q):
    send(client, q)
    return _send(client, q)
