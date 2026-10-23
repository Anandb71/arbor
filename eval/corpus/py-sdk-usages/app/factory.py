from openai import OpenAI


def make_client():
    return OpenAI(timeout=30)
