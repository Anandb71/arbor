import os

import google.generativeai as genai
from google import genai as genai_new

genai.configure(api_key=os.environ["GOOGLE_API_KEY"])
GEMINI_MODEL = "gemini-1.5-flash"


def legacy(prompt):
    model = genai.GenerativeModel(GEMINI_MODEL)
    return model.generate_content(prompt).text


def legacy_chat(prompt):
    chat = genai.GenerativeModel("gemini-pro").start_chat()
    return chat.send_message(prompt).text


def current(prompt):
    client = genai_new.Client()
    return client.models.generate_content(model="gemini-2.0-flash", contents=prompt).text
