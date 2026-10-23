"""Routing changed after construction, and a constructor imported two ways."""
from openai import OpenAI

try:
    from openai import AzureOpenAI as Maker
except ImportError:
    from openai import OpenAI as Maker

router = OpenAI()
router.base_url = "https://openrouter.ai/api/v1"
router.chat.completions.create(model="gpt-4", messages=[])

plain = OpenAI()
plain.chat.completions.create(model="gpt-4", messages=[])

either = Maker()
either.chat.completions.create(model="gpt-4", messages=[])
