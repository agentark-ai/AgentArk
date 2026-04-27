import ArrowDropDownRoundedIcon from "@mui/icons-material/ArrowDropDownRounded";
import { Box, Button, Chip, MenuItem, Stack, TextField, Typography } from "@mui/material";
import { useMemo, useState } from "react";
import { useUiStore } from "../../store/uiStore";
import { str, type JsonRecord } from "./pageHelpers";

export function normalizeProjectId(value: unknown): string {
  return str(value, "").trim();
}

export function withProjectScope(path: string, projectId: string): string {
  const normalizedProjectId = normalizeProjectId(projectId);
  if (!normalizedProjectId) return path;
  return `${path}${path.includes("?") ? "&" : "?"}project_id=${encodeURIComponent(normalizedProjectId)}`;
}

export function buildProjectNameById(projects: JsonRecord[]): Map<string, string> {
  const map = new Map<string, string>();
  for (const project of projects) {
    const id = normalizeProjectId(project.id);
    if (!id) continue;
    map.set(id, str(project.name, id));
  }
  return map;
}

export function projectScopeLabel(
  projectId: string,
  projectNameById: Map<string, string>,
): string {
  const normalizedProjectId = normalizeProjectId(projectId);
  if (!normalizedProjectId) return "Global workspace";
  return projectNameById.get(normalizedProjectId) || normalizedProjectId;
}

export function WorkspaceProjectScopeBar({
  activeProjectId,
  projects,
  onNavigateToView,
}: {
  activeProjectId: string;
  projects: JsonRecord[];
  onNavigateToView?: (view: string, replace?: boolean) => void;
}) {
  const setActiveProjectId = useUiStore((state) => state.setActiveProjectId);
  const projectNameById = useMemo(() => buildProjectNameById(projects), [projects]);
  const activeScopeLabel = projectScopeLabel(activeProjectId, projectNameById);
  const hasProjects = projects.length > 0;
  const [expanded, setExpanded] = useState(false);
  const scopeModeLabel = activeProjectId ? "Project" : "Global";
  const scopeSummary = activeProjectId
    ? "New chats, documents, and memories inherit this project."
    : "Everything stays global until you explicitly split it into a project.";

  return (
    <Box
      className={`workspace-scope-shell${expanded ? " is-expanded" : ""}`}
      sx={{ mb: 1.25, flexShrink: 0 }}
    >
      <Stack
        direction={{ xs: "column", md: "row" }}
        spacing={1}
        sx={{
          alignItems: { xs: "stretch", md: "center" },
          justifyContent: "space-between",
        }}
      >
        <Stack spacing={0.45} sx={{ minWidth: 0 }}>
          <Stack
            direction="row"
            spacing={0.75}
            useFlexGap
            sx={{ alignItems: "center", flexWrap: "wrap" }}
          >
            <Typography variant="overline" className="workspace-scope-kicker">
              Scope
            </Typography>
            <Chip
              size="small"
              label={scopeModeLabel}
              className="workspace-scope-chip"
              sx={{
                height: 22,
                borderRadius: 999,
                bgcolor: activeProjectId
                  ? "var(--ui-rgba-255-255-255-060)"
                  : "var(--ui-rgba-255-255-255-040)",
                borderColor: "var(--ui-rgba-255-255-255-080)",
              }}
            />
          </Stack>
          <Typography variant="body2" className="workspace-scope-title">
            {activeScopeLabel}
          </Typography>
          <Typography variant="caption" className="workspace-scope-caption">
            {scopeSummary}
          </Typography>
        </Stack>
        <Stack
          direction="row"
          spacing={0.75}
          useFlexGap
          sx={{
            alignItems: "center",
            flexWrap: "wrap",
            justifyContent: { xs: "flex-start", md: "flex-end" },
          }}
        >
          {onNavigateToView ? (
            <Button
              size="small"
              variant="outlined"
              onClick={() => onNavigateToView("projects")}
              sx={{ whiteSpace: "nowrap" }}
            >
              {hasProjects ? "Projects" : "New project"}
            </Button>
          ) : null}
          {hasProjects ? (
            <Button
              size="small"
              variant={expanded ? "contained" : "outlined"}
              onClick={() => setExpanded((previous) => !previous)}
              endIcon={
                <ArrowDropDownRoundedIcon
                  sx={{
                    transition: "transform 160ms ease",
                    transform: expanded ? "rotate(180deg)" : "rotate(0deg)",
                  }}
                />
              }
              sx={{ whiteSpace: "nowrap" }}
            >
              {expanded ? "Hide scope" : "Change scope"}
            </Button>
          ) : null}
        </Stack>
      </Stack>
      {hasProjects && expanded ? (
        <Stack
          direction={{ xs: "column", lg: "row" }}
          spacing={1}
          className="workspace-scope-controls"
          sx={{ alignItems: { xs: "stretch", lg: "center" } }}
        >
          <TextField
            fullWidth
            size="small"
            select
            label="Workspace scope"
            value={activeProjectId}
            onChange={(event) =>
              setActiveProjectId(normalizeProjectId(event.target.value))
            }
            sx={{ maxWidth: { lg: 340 } }}
          >
            <MenuItem value="">Global workspace</MenuItem>
            {projects.map((project) => {
              const id = normalizeProjectId(project.id);
              if (!id) return null;
              return (
                <MenuItem key={id} value={id}>
                  {projectScopeLabel(id, projectNameById)}
                </MenuItem>
              );
            })}
          </TextField>
          <Typography
            variant="caption"
            className="workspace-scope-caption"
            sx={{ maxWidth: 520 }}
          >
            Switch the active scope only when you want new chats, documents, and
            memory to stay inside that project.
          </Typography>
        </Stack>
      ) : null}
    </Box>
  );
}
