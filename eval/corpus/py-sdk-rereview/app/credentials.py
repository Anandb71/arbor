"""Credential shapes from the second review of #276. Nothing here may reach
the output: each value flows through a secret-like name or is itself
credential-shaped."""
from openai import OpenAI

client = OpenAI()


class ApiKeys:
    openai = "EVALRR2-CLASSATTR"


PASSWD = "EVALRR2-PASSWD"
CREDENTIALS = "EVALRR2-CREDENTIALS"
DATABASE_URL = "postgres://admin:EVALRR2pw@db.example/prod"
db_pwd = "EVALRR2-PWD"
LOOKUP = {"a": "b"}


def class_attr():
    return client.chat.completions.create(model=ApiKeys.openai, messages=[])


def passwd():
    return client.chat.completions.create(model=PASSWD, messages=[])


def credentials():
    return client.chat.completions.create(model=CREDENTIALS, messages=[])


def url_with_password():
    return client.chat.completions.create(model=DATABASE_URL, messages=[])


def pwd():
    return client.chat.completions.create(model=db_pwd, messages=[])


def refused_lookup_key():
    return client.chat.completions.create(model=LOOKUP.get("sk-proj-tokenEVALRR2KEY00000000", "gpt-4"), messages=[])


def nested_credentials():
    return client.chat.completions.create(
        model="gpt-4o", messages=[], extra_body={"credentials": {"model": "EVALRR2-NESTED"}}
    )
