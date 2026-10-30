from .claude import ClaudeSampler
from .sampler import Sampler, ask

MODEL = "gpt-4.1-mini"

SAMPLERS = {
    "o1": Sampler(model="o1"),
    "mini": Sampler(model=MODEL),
    "default": Sampler(),
    "claude": ClaudeSampler("claude-3-5-haiku-20241022"),
}


def quick(question):
    return ask(question, model="gpt-3.5-turbo")
