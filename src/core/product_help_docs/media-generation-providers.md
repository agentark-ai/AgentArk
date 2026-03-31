# Media generation providers

Path: `Settings > Media`.

Use this area to configure image and video generation providers and their API keys.

What is here:

- provider API keys for supported media backends
- default image provider
- image model
- fallback image provider
- default video provider
- fallback video provider

Typical setup:

1. Open `Settings > Media`.
2. Save the API key for the provider you want to use.
3. Set the default image provider and image model if you want image generation.
4. Set the default video provider if you want video generation.
5. Optionally set fallbacks so AgentArk can retry on another provider.
6. Save settings.

Verification:

- `Settings > Media` should show configured providers instead of `No media providers`.
- Image or video tasks should stop failing for missing provider credentials.

Common issues:

- A provider key exists, but no default provider was selected.
- The default provider was chosen, but the model field is blank or invalid.
- The user saved only fallback providers and expected them to behave like defaults.
