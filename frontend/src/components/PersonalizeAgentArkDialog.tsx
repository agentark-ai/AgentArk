import {
  Alert,
  Autocomplete,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  MenuItem,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { api } from "../api/client";

const TIMEZONE_OPTIONS = [
  "UTC",
  "America/New_York",
  "America/Chicago",
  "America/Denver",
  "America/Los_Angeles",
  "America/Phoenix",
  "America/Toronto",
  "America/Vancouver",
  "Europe/London",
  "Europe/Paris",
  "Europe/Berlin",
  "Asia/Dubai",
  "Asia/Kolkata",
  "Asia/Singapore",
  "Asia/Tokyo",
  "Australia/Sydney",
];

const RESPONSE_STYLE_OPTIONS = [
  { value: "concise", label: "Concise" },
  { value: "professional", label: "Professional" },
  { value: "friendly", label: "Friendly" },
  { value: "technical", label: "Technical" },
  { value: "casual", label: "Casual" },
  { value: "creative", label: "Creative" },
];

function str(value: unknown, fallback = ""): string {
  return typeof value === "string" ? value : fallback;
}

type Props = {
  open: boolean;
  profile?: Record<string, unknown>;
  onClose: () => void;
};

export function PersonalizeAgentArkDialog({ open, profile, onClose }: Props) {
  const queryClient = useQueryClient();
  const detectedTimezone = useMemo(() => {
    try {
      return Intl.DateTimeFormat().resolvedOptions().timeZone || "";
    } catch {
      return "";
    }
  }, []);
  const [preferredName, setPreferredName] = useState("");
  const [timezone, setTimezone] = useState("");
  const [responseStyle, setResponseStyle] = useState("concise");
  const [priorityFocus, setPriorityFocus] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setPreferredName(str(profile?.name, ""));
    setTimezone(str(profile?.timezone, detectedTimezone));
    setResponseStyle(str(profile?.tone, "concise") || "concise");
    setPriorityFocus(
      str(profile?.priority_focus, str(profile?.preferences, "")),
    );
    setError(null);
  }, [detectedTimezone, open, profile]);

  const saveMutation = useMutation({
    mutationFn: async () =>
      api.rawPost("/profile/onboarding", {
        preferred_name: preferredName.trim(),
        timezone: timezone.trim(),
        tone: responseStyle.trim(),
        priority_focus: priorityFocus.trim(),
      }),
    onSuccess: async () => {
      setError(null);
      await queryClient.invalidateQueries({ queryKey: ["profile"] });
      await queryClient.invalidateQueries({ queryKey: ["settings"] });
      await queryClient.invalidateQueries({
        queryKey: ["status-page-profile"],
      });
      onClose();
    },
    onError: (nextError) => {
      const message =
        nextError instanceof Error
          ? nextError.message
          : String(nextError || "Failed to save personalization.");
      setError(message);
    },
  });
  const dismissMutation = useMutation({
    mutationFn: async () => api.rawPost("/profile/onboarding/dismiss"),
    onSuccess: async () => {
      setError(null);
      await queryClient.invalidateQueries({ queryKey: ["profile"] });
      await queryClient.invalidateQueries({ queryKey: ["settings"] });
      await queryClient.invalidateQueries({
        queryKey: ["status-page-profile"],
      });
      onClose();
    },
    onError: (nextError) => {
      const message =
        nextError instanceof Error
          ? nextError.message
          : String(nextError || "Failed to dismiss personalization.");
      setError(message);
    },
  });

  const saveDisabled =
    saveMutation.isPending ||
    dismissMutation.isPending ||
    !preferredName.trim() ||
    !timezone.trim() ||
    !responseStyle.trim();

  return (
    <Dialog
      open={open}
      onClose={
        saveMutation.isPending || dismissMutation.isPending
          ? undefined
          : (_event, _reason) => onClose()
      }
      fullWidth
      maxWidth="sm"
      slotProps={{
        paper: {
          sx: {
            borderRadius: 2,
            border: "1px solid var(--ui-rgba-255-255-255-080)",
            background:
              "linear-gradient(160deg, var(--ui-rgba-24-24-28-980), var(--ui-rgba-15-15-18-950))",
          },
        },
      }}
    >
      <DialogTitle sx={{ pb: 0.5 }}>
        <Typography variant="h5" sx={{ fontWeight: 700 }}>Personalize AgentArk</Typography>
        <Typography variant="body2" sx={{ color: "text.secondary", mt: 0.5 }}>
          A short first-run pass so briefs, replies, and follow-up start in the right shape.
        </Typography>
      </DialogTitle>
      <DialogContent>
        <Stack spacing={2} sx={{ pt: 1 }}>
          {error ? <Alert severity="error">{error}</Alert> : null}
          <TextField
            label="What should AgentArk call you?"
            value={preferredName}
            onChange={(event) => setPreferredName(event.target.value)}
            fullWidth
            placeholder="e.g. Ava"
          />
          <Autocomplete
            freeSolo
            options={TIMEZONE_OPTIONS}
            value={timezone}
            onChange={(_, value) => setTimezone(String(value ?? ""))}
            inputValue={timezone}
            onInputChange={(_, value) => setTimezone(value)}
            renderInput={(params) => (
              <TextField
                {...params}
                label="Timezone"
                fullWidth
                placeholder="e.g. America/New_York"
              />
            )}
          />
          <TextField
            label="Response style"
            select
            value={responseStyle}
            onChange={(event) => setResponseStyle(event.target.value)}
            fullWidth
          >
            {RESPONSE_STYLE_OPTIONS.map((option) => (
              <MenuItem key={option.value} value={option.value}>
                {option.label}
              </MenuItem>
            ))}
          </TextField>
          <TextField
            label="What should AgentArk prioritize first? (optional)"
            value={priorityFocus}
            onChange={(event) => setPriorityFocus(event.target.value)}
            fullWidth
            multiline
            minRows={3}
            placeholder="e.g. Daily brief, urgent inbox triage, and task follow-up"
          />
          <Typography variant="caption" sx={{ color: "text.secondary" }}>
            You can change these later in Settings and Memory.
          </Typography>
        </Stack>
      </DialogContent>
      <DialogActions sx={{ px: 3, pb: 2.5 }}>
        <Button
          onClick={() => dismissMutation.mutate()}
          disabled={saveMutation.isPending || dismissMutation.isPending}
        >
          Later
        </Button>
        <Button
          variant="contained"
          onClick={() => saveMutation.mutate()}
          disabled={saveDisabled}
        >
          {saveMutation.isPending ? "Saving..." : "Save Personalization"}
        </Button>
      </DialogActions>
    </Dialog>
  );
}
