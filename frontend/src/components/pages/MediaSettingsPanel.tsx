import {
  Box,
  Divider,
  MenuItem,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import Grid2 from "@mui/material/Grid";
import type { ReactNode } from "react";

const MEDIA_PROVIDER_OPTIONS = [
  { value: "replicate", label: "Replicate" },
  { value: "fal", label: "FAL.ai" },
  { value: "stability_ai", label: "Stability AI" },
  { value: "together", label: "Together.ai" },
  { value: "openai_dalle", label: "OpenAI Images" },
  { value: "openai_sora", label: "OpenAI Sora" },
  { value: "google_gemini", label: "Google Gemini" },
  { value: "google_veo", label: "Google Veo" },
  { value: "runway", label: "Runway" },
  { value: "luma", label: "Luma" },
] as const;

const MEDIA_IMAGE_PROVIDER_IDS = new Set([
  "replicate",
  "fal",
  "stability_ai",
  "together",
  "openai_dalle",
  "google_gemini",
]);

const MEDIA_VIDEO_PROVIDER_IDS = new Set([
  "replicate",
  "fal",
  "stability_ai",
  "openai_sora",
  "google_veo",
  "runway",
  "luma",
]);

const MEDIA_IMAGE_PROVIDER_OPTIONS = MEDIA_PROVIDER_OPTIONS.filter((provider) =>
  MEDIA_IMAGE_PROVIDER_IDS.has(provider.value),
);

const MEDIA_VIDEO_PROVIDER_OPTIONS = MEDIA_PROVIDER_OPTIONS.filter((provider) =>
  MEDIA_VIDEO_PROVIDER_IDS.has(provider.value),
);

export type MediaSettingsFormFields = {
  default_image_provider: string;
  image_model: string;
  fallback_image_provider: string;
  default_video_provider: string;
  fallback_video_provider: string;
  media_provider_keys_json: string;
  media_key_replicate: string;
  media_key_fal: string;
  media_key_stability_ai: string;
  media_key_together: string;
  media_key_openai_dalle: string;
  media_key_google_gemini: string;
  media_key_runway: string;
  media_key_luma: string;
  media_base_url_replicate: string;
  media_base_url_fal: string;
  media_base_url_stability_ai: string;
  media_base_url_together: string;
  media_base_url_openai_dalle: string;
  media_base_url_openai_sora: string;
  media_base_url_google_gemini: string;
  media_base_url_google_veo: string;
  media_base_url_runway: string;
  media_base_url_luma: string;
};

type MediaSettingsPanelProps = {
  form: MediaSettingsFormFields;
  setField: <K extends keyof MediaSettingsFormFields>(
    key: K,
    value: MediaSettingsFormFields[K],
  ) => void;
  configuredProviders: string[];
  renderSettingsSectionIntro: (args: {
    eyebrow: string;
    title: string;
    description: string;
  }) => ReactNode;
};

export function MediaSettingsPanel({
  form,
  setField,
  configuredProviders,
  renderSettingsSectionIntro,
}: MediaSettingsPanelProps) {
  return (
    <Grid2 container spacing={1.5} sx={{ alignItems: "stretch" }}>
      <Grid2 size={{ xs: 12, lg: 6 }} sx={{ display: "flex" }}>
        <Box sx={{ minHeight: 0, width: "100%" }}>
          {renderSettingsSectionIntro({
            eyebrow: "Media",
            title: "Provider Keys",
            description:
              "Keys stay encrypted at rest. Leave fields blank to keep any existing saved secrets.",
          })}
          <Stack spacing={1.2} sx={{ mt: 1 }}>
            <TextField
              label="Replicate API Key"
              value={form.media_key_replicate}
              onChange={(e) => setField("media_key_replicate", e.target.value)}
              fullWidth
              size="small"
              type="password"
            />
            <TextField
              label="FAL API Key"
              value={form.media_key_fal}
              onChange={(e) => setField("media_key_fal", e.target.value)}
              fullWidth
              size="small"
              type="password"
            />
            <TextField
              label="Stability AI API Key"
              value={form.media_key_stability_ai}
              onChange={(e) => setField("media_key_stability_ai", e.target.value)}
              fullWidth
              size="small"
              type="password"
            />
            <TextField
              label="Together API Key"
              value={form.media_key_together}
              onChange={(e) => setField("media_key_together", e.target.value)}
              fullWidth
              size="small"
              type="password"
            />
            <TextField
              label="OpenAI API Key"
              value={form.media_key_openai_dalle}
              onChange={(e) => setField("media_key_openai_dalle", e.target.value)}
              fullWidth
              size="small"
              type="password"
            />
            <TextField
              label="Google AI API Key"
              value={form.media_key_google_gemini}
              onChange={(e) => setField("media_key_google_gemini", e.target.value)}
              fullWidth
              size="small"
              type="password"
            />
            <TextField
              label="Runway API Key"
              value={form.media_key_runway}
              onChange={(e) => setField("media_key_runway", e.target.value)}
              fullWidth
              size="small"
              type="password"
            />
            <TextField
              label="Luma API Key"
              value={form.media_key_luma}
              onChange={(e) => setField("media_key_luma", e.target.value)}
              fullWidth
              size="small"
              type="password"
            />
          </Stack>
          <Divider sx={{ my: 2 }} />
          <Typography variant="caption" sx={{ color: "text.secondary" }}>
            Detected configured providers:{" "}
            {configuredProviders.length ? configuredProviders.join(", ") : "(none detected)"}
          </Typography>
        </Box>
      </Grid2>

      <Grid2 size={{ xs: 12, lg: 6 }} sx={{ display: "flex" }}>
        <Box className="list-shell" sx={{ minHeight: 0, width: "100%" }}>
          {renderSettingsSectionIntro({
            eyebrow: "Media",
            title: "Defaults",
            description:
              "Choose the default and fallback providers AgentArk uses for image and video generation.",
          })}
          <Stack spacing={1.2}>
            <TextField
              label="Default Image Provider"
              value={form.default_image_provider}
              onChange={(e) => setField("default_image_provider", e.target.value)}
              fullWidth
              size="small"
              select
            >
              <MenuItem value="">Auto</MenuItem>
              {MEDIA_IMAGE_PROVIDER_OPTIONS.map((provider) => (
                <MenuItem key={provider.value} value={provider.value}>
                  {provider.label}
                </MenuItem>
              ))}
            </TextField>
            <TextField
              label="Image Model"
              value={form.image_model}
              onChange={(e) => setField("image_model", e.target.value)}
              fullWidth
              size="small"
            />
            <TextField
              label="Fallback Image Provider"
              value={form.fallback_image_provider}
              onChange={(e) => setField("fallback_image_provider", e.target.value)}
              fullWidth
              size="small"
              select
            >
              <MenuItem value="">None</MenuItem>
              {MEDIA_IMAGE_PROVIDER_OPTIONS.map((provider) => (
                <MenuItem key={provider.value} value={provider.value}>
                  {provider.label}
                </MenuItem>
              ))}
            </TextField>
            <TextField
              label="Default Video Provider"
              value={form.default_video_provider}
              onChange={(e) => setField("default_video_provider", e.target.value)}
              fullWidth
              size="small"
              select
            >
              <MenuItem value="">Auto</MenuItem>
              {MEDIA_VIDEO_PROVIDER_OPTIONS.map((provider) => (
                <MenuItem key={provider.value} value={provider.value}>
                  {provider.label}
                </MenuItem>
              ))}
            </TextField>
            <TextField
              label="Fallback Video Provider"
              value={form.fallback_video_provider}
              onChange={(e) => setField("fallback_video_provider", e.target.value)}
              fullWidth
              size="small"
              select
            >
              <MenuItem value="">None</MenuItem>
              {MEDIA_VIDEO_PROVIDER_OPTIONS.map((provider) => (
                <MenuItem key={provider.value} value={provider.value}>
                  {provider.label}
                </MenuItem>
              ))}
            </TextField>
          </Stack>
          <Divider sx={{ my: 2 }} />
          {renderSettingsSectionIntro({
            eyebrow: "Media",
            title: "Provider Endpoints",
            description:
              "Override only when using a compatible proxy or self-hosted endpoint for the selected provider API.",
          })}
          <Stack spacing={1.2} sx={{ mt: 1 }}>
            <TextField
              label="OpenAI Endpoint"
              value={form.media_base_url_openai_dalle}
              onChange={(e) => {
                setField("media_base_url_openai_dalle", e.target.value);
                setField("media_base_url_openai_sora", e.target.value);
              }}
              fullWidth
              size="small"
            />
            <TextField
              label="Google AI Endpoint"
              value={form.media_base_url_google_gemini}
              onChange={(e) => {
                setField("media_base_url_google_gemini", e.target.value);
                setField("media_base_url_google_veo", e.target.value);
              }}
              fullWidth
              size="small"
            />
            <TextField
              label="Replicate Endpoint"
              value={form.media_base_url_replicate}
              onChange={(e) => setField("media_base_url_replicate", e.target.value)}
              fullWidth
              size="small"
            />
            <TextField
              label="FAL Endpoint"
              value={form.media_base_url_fal}
              onChange={(e) => setField("media_base_url_fal", e.target.value)}
              fullWidth
              size="small"
            />
            <TextField
              label="Stability AI Endpoint"
              value={form.media_base_url_stability_ai}
              onChange={(e) => setField("media_base_url_stability_ai", e.target.value)}
              fullWidth
              size="small"
            />
            <TextField
              label="Together Endpoint"
              value={form.media_base_url_together}
              onChange={(e) => setField("media_base_url_together", e.target.value)}
              fullWidth
              size="small"
            />
            <TextField
              label="Runway Endpoint"
              value={form.media_base_url_runway}
              onChange={(e) => setField("media_base_url_runway", e.target.value)}
              fullWidth
              size="small"
            />
            <TextField
              label="Luma Endpoint"
              value={form.media_base_url_luma}
              onChange={(e) => setField("media_base_url_luma", e.target.value)}
              fullWidth
              size="small"
            />
          </Stack>
          <Divider sx={{ my: 2 }} />
          {renderSettingsSectionIntro({
            eyebrow: "Media",
            title: "Advanced JSON",
            description:
              "Optional raw provider-to-key mapping when you need explicit JSON control over media credentials.",
          })}
          <TextField
            label="media_providers JSON"
            value={form.media_provider_keys_json}
            onChange={(e) => setField("media_provider_keys_json", e.target.value)}
            fullWidth
            multiline
            minRows={6}
            sx={{ mt: 1 }}
          />
        </Box>
      </Grid2>
    </Grid2>
  );
}
