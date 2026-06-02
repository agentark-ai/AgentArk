import {
  Alert,
  Box,
  Button,
  Chip,
  Dialog,
  DialogContent,
  DialogTitle,
  IconButton,
  Stack,
  Typography,
} from "@mui/material";
import CloseRoundedIcon from "@mui/icons-material/CloseRounded";
import OpenInNewRoundedIcon from "@mui/icons-material/OpenInNewRounded";
import type { ReactNode } from "react";

const REDIRECT_URI = "http://localhost:8990/oauth/callback";

type ApiLink = { label: string; url: string };

const API_LINKS: ApiLink[] = [
  {
    label: "Gmail API",
    url: "https://console.cloud.google.com/apis/library/gmail.googleapis.com",
  },
  {
    label: "Google Calendar API",
    url: "https://console.cloud.google.com/apis/library/calendar-json.googleapis.com",
  },
  {
    label: "Google Drive API",
    url: "https://console.cloud.google.com/apis/library/drive.googleapis.com",
  },
  {
    label: "Google Docs API",
    url: "https://console.cloud.google.com/apis/library/docs.googleapis.com",
  },
  {
    label: "Google Sheets API",
    url: "https://console.cloud.google.com/apis/library/sheets.googleapis.com",
  },
  {
    label: "Google Chat API",
    url: "https://console.cloud.google.com/apis/library/chat.googleapis.com",
  },
  {
    label: "Admin SDK API",
    url: "https://console.cloud.google.com/apis/library/admin.googleapis.com",
  },
];

type Step = {
  n: number;
  title: string;
  body: ReactNode;
  primary?: { url: string; label: string };
};

function openExternal(url: string) {
  window.open(url, "_blank", "noopener,noreferrer");
}

function ExternalLink({
  url,
  label,
  primary = false,
}: {
  url: string;
  label: string;
  primary?: boolean;
}) {
  return (
    <Button
      variant={primary ? "outlined" : "text"}
      size="small"
      endIcon={<OpenInNewRoundedIcon fontSize="small" />}
      onClick={() => openExternal(url)}
      sx={
        primary
          ? undefined
          : { minWidth: 0, px: 0.75, py: 0.25, textTransform: "none" }
      }
    >
      {label}
    </Button>
  );
}

const STEPS: Step[] = [
  {
    n: 1,
    title: "Pick or create a Google Cloud project",
    body: (
      <Typography variant="body2" color="text.secondary">
        The project is just a container for the OAuth credentials. Pick any name —
        there is no charge. If you already have a project you don&apos;t mind reusing,
        select it from the project picker at the top of the Cloud Console and
        skip to step 2.
      </Typography>
    ),
    primary: {
      url: "https://console.cloud.google.com/projectcreate",
      label: "Open Cloud Console",
    },
  },
  {
    n: 2,
    title: "Enable the APIs you want AgentArk to use",
    body: (
      <Stack spacing={1}>
        <Typography variant="body2" color="text.secondary">
          Click each API you plan to use, then press <strong>ENABLE</strong> on its
          page. You only need the ones you&apos;ll actually connect — you can come back
          and enable more later.
        </Typography>
        <Box sx={{ display: "flex", flexWrap: "wrap", gap: 0.5 }}>
          {API_LINKS.map((api) => (
            <ExternalLink key={api.url} url={api.url} label={api.label} />
          ))}
        </Box>
      </Stack>
    ),
  },
  {
    n: 3,
    title: "Configure the OAuth consent screen",
    body: (
      <Stack spacing={1.25}>
        <Typography variant="body2" color="text.secondary">
          This is the screen Google shows when AgentArk asks for permission. Fill
          in once per project.
        </Typography>
        <Box
          component="ul"
          sx={{
            pl: 2.5,
            m: 0,
            color: "text.secondary",
            "& li": { mb: 0.5, fontSize: "0.875rem" },
          }}
        >
          <li>
            User type: <strong>External</strong>{" "}
            <em>(Google Workspace admins can pick Internal for less friction)</em>
          </li>
          <li>
            App name: anything — e.g. &quot;My AgentArk&quot;
          </li>
          <li>
            User support email + developer contact: your own email
          </li>
          <li>
            Scopes: <em>skip this step</em>. AgentArk requests them at sign-in time.
          </li>
          <li>
            <strong>Test users:</strong> add your own Gmail address. This is the
            step most people miss.
          </li>
        </Box>
        <Alert severity="warning" sx={{ alignItems: "flex-start" }}>
          Until you add yourself as a Test user, Google blocks the consent screen.
          If you ever see &quot;Access blocked: this app&apos;s request is invalid&quot;,
          this is almost always why.
        </Alert>
      </Stack>
    ),
    primary: {
      url: "https://console.cloud.google.com/apis/credentials/consent",
      label: "Open consent screen",
    },
  },
  {
    n: 4,
    title: "Create the OAuth client",
    body: (
      <Stack spacing={1.25}>
        <Typography variant="body2" color="text.secondary">
          The actual client credentials AgentArk will use.
        </Typography>
        <Box
          component="ul"
          sx={{
            pl: 2.5,
            m: 0,
            color: "text.secondary",
            "& li": { mb: 0.5, fontSize: "0.875rem" },
          }}
        >
          <li>
            Click <strong>+ CREATE CREDENTIALS</strong> →{" "}
            <strong>OAuth client ID</strong>
          </li>
          <li>
            Application type: <strong>Web application</strong>
          </li>
          <li>Name: anything, e.g. &quot;AgentArk local&quot;</li>
          <li>
            Authorized redirect URIs: click <strong>+ ADD URI</strong> and paste:
          </li>
        </Box>
        <Chip
          label={REDIRECT_URI}
          variant="outlined"
          sx={{
            fontFamily: "var(--font-mono)",
            fontSize: "0.82rem",
            alignSelf: "flex-start",
            cursor: "pointer",
            borderColor: "rgba(120, 242, 176, 0.42)",
            color: "rgba(186, 247, 228, 0.96)",
            background: "rgba(120, 242, 176, 0.08)",
          }}
          onClick={() =>
            void navigator.clipboard?.writeText(REDIRECT_URI).catch(() => {})
          }
          title="Click to copy"
        />
        <Typography variant="caption" color="text.secondary">
          Click the chip to copy. Use <strong>Web application</strong>, not Desktop
          — Google has deprecated the Desktop loopback flow for sensitive scopes.
        </Typography>
      </Stack>
    ),
    primary: {
      url: "https://console.cloud.google.com/apis/credentials",
      label: "Open Credentials",
    },
  },
  {
    n: 5,
    title: "Download the credentials JSON",
    body: (
      <Typography variant="body2" color="text.secondary">
        After clicking <strong>CREATE</strong>, you&apos;ll see &quot;OAuth client
        created&quot;. Click <strong>DOWNLOAD JSON</strong>. Save the file anywhere
        — you only need its contents.
      </Typography>
    ),
  },
  {
    n: 6,
    title: "Paste in AgentArk",
    body: (
      <Typography variant="body2" color="text.secondary">
        Open the downloaded JSON in any text editor, copy the full contents, paste
        into the <strong>Google OAuth Client JSON</strong> field on the setup page,
        and click <strong>Save Google OAuth</strong>. That&apos;s it — AgentArk
        will open the Google consent screen the next time you connect a Google
        service.
      </Typography>
    ),
  },
];

export function GoogleWorkspaceSetupGuide({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  return (
    <Dialog open={open} onClose={onClose} fullScreen>
      <DialogTitle
        sx={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          gap: 2,
          borderBottom: "1px solid var(--ui-rgba-112-153-201-120)",
        }}
      >
        <Box>
          <Typography variant="h6" component="div">
            Google Workspace setup guide
          </Typography>
          <Typography variant="caption" color="text.secondary">
            Six steps, roughly 5–10 minutes the first time you do this.
          </Typography>
        </Box>
        <IconButton size="small" onClick={onClose} aria-label="Close setup guide">
          <CloseRoundedIcon />
        </IconButton>
      </DialogTitle>
      <DialogContent
        sx={{
          py: 3,
          px: { xs: 2, sm: 3 },
        }}
      >
        <Stack
          spacing={2.5}
          sx={{ width: "100%", maxWidth: 880, mx: "auto" }}
        >
          <Alert severity="info">
            AgentArk runs locally and never holds your credentials on a remote
            server, so you create your own Google OAuth client once and paste it
            here. After that, every Google service is a one-click sign-in.
          </Alert>
          {STEPS.map((step) => (
            <Box
              key={step.n}
              sx={{
                border: "1px solid var(--ui-rgba-112-153-201-120)",
                borderRadius: 2,
                p: 2,
                background: "var(--ui-rgba-8-18-32-460)",
              }}
            >
              <Stack
                direction="row"
                spacing={2}
                sx={{ alignItems: "flex-start" }}
              >
                <Box
                  sx={{
                    flex: "0 0 auto",
                    width: 30,
                    height: 30,
                    borderRadius: "50%",
                    border: "1px solid rgba(120, 242, 176, 0.42)",
                    background: "rgba(120, 242, 176, 0.12)",
                    color: "rgba(186, 247, 228, 0.98)",
                    display: "inline-flex",
                    alignItems: "center",
                    justifyContent: "center",
                    fontWeight: 700,
                    fontSize: "0.92rem",
                    lineHeight: 1,
                  }}
                >
                  {step.n}
                </Box>
                <Stack spacing={1.25} sx={{ flex: 1, minWidth: 0 }}>
                  <Typography variant="subtitle1" sx={{ fontWeight: 620 }}>
                    {step.title}
                  </Typography>
                  {step.body}
                  {step.primary ? (
                    <Box>
                      <ExternalLink
                        url={step.primary.url}
                        label={step.primary.label}
                        primary
                      />
                    </Box>
                  ) : null}
                </Stack>
              </Stack>
            </Box>
          ))}
          <Stack direction="row" spacing={1.5} sx={{ pt: 0.5, justifyContent: "center" }}>
            <Button variant="contained" onClick={onClose}>
              Back to setup
            </Button>
          </Stack>
        </Stack>
      </DialogContent>
    </Dialog>
  );
}
