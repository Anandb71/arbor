"""Token prices per model. Not a call site: nothing here is a usage."""
import openai

PRICES_PER_1K = {
    "gpt-3.5-turbo": 0.0005,
    "gpt-4": 0.03,
    "claude-3-5-sonnet-20241022": 0.003,
    "gemini-1.5-flash": 0.000075,
}
RETIRED = ["gpt-3.5-turbo", "gpt-4", "claude-3-5-sonnet-20241022"]


def is_retired(model):
    return model in RETIRED


def sdk_version():
    return openai.__version__
