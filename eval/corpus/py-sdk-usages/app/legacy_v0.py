import openai

openai.api_type = "azure"
openai.api_base = "https://example.openai.azure.com/"
openai.api_version = "2023-05-15"
openai.api_key = "sk-test-NOT-A-REAL-KEY-0003"


def old_chat(prompt):
    return openai.ChatCompletion.create(engine="gpt-35-turbo", messages=[{"role": "user", "content": prompt}])


def old_complete(prompt):
    return openai.Completion.create(engine="text-davinci-003", prompt=prompt)
