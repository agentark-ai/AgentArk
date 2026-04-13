import {
  Box,
  Button,
  Chip,
  Stack,
  Typography,
} from "@mui/material";
import { useMemo, useState } from "react";
import { NativeWorkspace, type WorkspaceView } from "./NativeWorkspace";

type LibraryView = Extract<WorkspaceView, "skills" | "documents" | "apps">;

type Props = {
  autoRefresh: boolean;
  showAdvanced: boolean;
  onNavigateToView: (view: string, replace?: boolean) => void;
};

const LIBRARY_VIEWS: Array<{ view: LibraryView; label: string; detail: string }> = [
  {
    view: "skills",
    label: "Skills",
    detail: "Import, inspect, and manage capabilities.",
  },
  {
    view: "documents",
    label: "Documents",
    detail: "Project knowledge, uploads, and indexed context.",
  },
  {
    view: "apps",
    label: "Apps",
    detail: "Built outputs, deployed surfaces, and public links.",
  },
];

export function LibraryPane({ autoRefresh, showAdvanced, onNavigateToView }: Props) {
  const [activeView, setActiveView] = useState<LibraryView>("skills");
  const activeMeta = useMemo(
    () => LIBRARY_VIEWS.find((entry) => entry.view === activeView) || LIBRARY_VIEWS[0],
    [activeView]
  );

  return (
    <Box className="library-shell">
      <Box className="library-hero">
        <Typography variant="overline" className="workspace-shell-kicker">
          Library
        </Typography>
        <Typography variant="h4" sx={{ fontWeight: 700, letterSpacing: 0, mb: 0.45 }}>
          Reusable knowledge, capabilities, and artifacts.
        </Typography>
        <Typography
          variant="body2"
          sx={{
            color: "text.secondary",
            maxWidth: 860
          }}>
          Keep imported skills, indexed documents, and built apps together. This is the reusable substrate the workspace can
          draw from while tasks are running.
        </Typography>
        <Stack
          direction="row"
          spacing={0.75}
          useFlexGap
          sx={{
            flexWrap: "wrap",
            mt: 1.2
          }}>
          {LIBRARY_VIEWS.map((entry) => (
            <Button
              key={entry.view}
              size="small"
              variant={activeView === entry.view ? "contained" : "outlined"}
              onClick={() => setActiveView(entry.view)}
            >
              {entry.label}
            </Button>
          ))}
          <Button
            size="small"
            variant="text"
            onClick={() => onNavigateToView("projects")}
          >
            Projects
          </Button>
        </Stack>
        <Stack
          direction="row"
          spacing={0.75}
          useFlexGap
          sx={{
            flexWrap: "wrap",
            mt: 1
          }}>
          <Chip size="small" label={activeMeta.label} color="primary" />
          <Chip size="small" label={activeMeta.detail} />
        </Stack>
      </Box>
      <Box className="library-stage">
        <NativeWorkspace
          view={activeView}
          autoRefresh={autoRefresh}
          showAdvanced={showAdvanced}
          onNavigateToView={onNavigateToView}
        />
      </Box>
    </Box>
  );
}
