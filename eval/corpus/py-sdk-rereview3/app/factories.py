"""A client constructor chosen at run time against something that is not
one: what it builds is ambiguous."""
import functools
import os

from openai import AzureOpenAI, OpenAI


def build():
    return OpenAI(base_url="https://gateway.example/v1")


make = AzureOpenAI if os.getenv("AZ") else build
c1 = make()
c1.chat.completions.create(model="gpt-4", messages=[])

make2 = OpenAI if os.getenv("D") else make_client  # noqa: F821
c2 = make2()
c2.chat.completions.create(model="gpt-4", messages=[])

make3 = AzureOpenAI if os.getenv("AZ") else functools.partial(OpenAI, base_url="https://gateway.example/v1")
c3 = make3()
c3.chat.completions.create(model="gpt-4", messages=[])

try:
    from openai import AzureOpenAI as Maker
except ImportError:
    from mylib import FakeClient as Maker
c4 = Maker()
c4.chat.completions.create(model="gpt-4", messages=[])
