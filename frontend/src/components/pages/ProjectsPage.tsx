import {
  Alert,
  Box,
  Button,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  FormControlLabel,
  Stack,
  Switch,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
  TextField,
  Typography,
} from "@mui/material";
import Grid2 from "@mui/material/Grid";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { api } from "../../api/client";
import { useUiStore } from "../../store/uiStore";
import { WorkspacePageHeader, WorkspacePageShell } from "../WorkspacePage";
import {
  buildProjectNameById,
  normalizeProjectId,
  projectScopeLabel,
  withProjectScope,
} from "./projectScope";
import {
  asRecord,
  errMessage,
  pickRecords,
  str,
  type JsonRecord,
} from "./pageHelpers";
import { humanTs, QueryTable, RowOpsMenu } from "./workspaceUiBits";

const REFRESH_MS = 8000;

type ProjectsPageProps = {
  autoRefresh: boolean;
  projects: JsonRecord[];
  activeProjectId: string;
  onOpenProjectWorkspace: (projectId: string) => void;
};

export default function ProjectsPage({
  autoRefresh,
  projects,
  activeProjectId,
  onOpenProjectWorkspace,
}: ProjectsPageProps) {
  const queryClient = useQueryClient();
  const setActiveProjectId = useUiStore((s) => s.setActiveProjectId);
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [createOpen, setCreateOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selectedProject, setSelectedProject] = useState<JsonRecord | null>(
    null,
  );
  const [deleteProject, setDeleteProject] = useState<JsonRecord | null>(null);
  const [deleteConfirm, setDeleteConfirm] = useState("");
  const [editForm, setEditForm] = useState({
    name: "",
    description: "",
    system_prompt: "",
    personality: "",
    tools_filter: "",
    active: true,
  });

  const conversationsQ = useQuery({
    queryKey: ["projects-conversations"],
    queryFn: () => api.rawGet("/conversations?limit=100"),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });
  const projectNameById = useMemo(
    () => buildProjectNameById(projects),
    [projects],
  );
  const activeScopeLabel = projectScopeLabel(activeProjectId, projectNameById);
  const scopedConversationPath = useMemo(
    () => withProjectScope("/conversations", activeProjectId),
    [activeProjectId],
  );

  const createMutation = useMutation({
    mutationFn: () =>
      api.rawPost("/projects", {
        name: name.trim(),
        description: description.trim(),
      }),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["workspace-projects"] });
      await queryClient.invalidateQueries({
        queryKey: ["projects-conversations"],
      });
    },
  });
  const deleteMutation = useMutation({
    mutationFn: (id: string) =>
      api.rawDelete(`/projects/${encodeURIComponent(id)}`),
    onSuccess: async (_data, deletedId) => {
      await queryClient.invalidateQueries({ queryKey: ["workspace-projects"] });
      await queryClient.invalidateQueries({
        queryKey: ["projects-conversations"],
      });
      await queryClient.invalidateQueries({ queryKey: ["documents-manager"] });
      await queryClient.invalidateQueries({ queryKey: ["memory-stats"] });
      await queryClient.invalidateQueries({ queryKey: ["memory-facts"] });
      await queryClient.invalidateQueries({ queryKey: ["memory-preferences"] });
      await queryClient.invalidateQueries({ queryKey: ["memory-user-data"] });
      await queryClient.invalidateQueries({ queryKey: ["memory-knowledge"] });
      await queryClient.invalidateQueries({ queryKey: ["chat-conversations"] });
      if (deletedId === activeProjectId) {
        setActiveProjectId("");
      }
      setDeleteProject(null);
      setDeleteConfirm("");
    },
  });
  const updateMutation = useMutation({
    mutationFn: (payload: { id: string; body: Record<string, unknown> }) =>
      api.rawPut(`/projects/${encodeURIComponent(payload.id)}`, payload.body),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["workspace-projects"] });
      await queryClient.invalidateQueries({
        queryKey: ["projects-conversations"],
      });
      await queryClient.invalidateQueries({ queryKey: ["chat-conversations"] });
      setSelectedProject(null);
    },
  });

  const conversations = pickRecords(conversationsQ.data, "conversations");
  const counts = useMemo(() => {
    const map = new Map<string, number>();
    conversations.forEach((conv) => {
      const pid = str(conv.project_id, "");
      if (!pid) return;
      map.set(pid, (map.get(pid) || 0) + 1);
    });
    return map;
  }, [conversations]);

  return (
    <WorkspacePageShell spacing={1.5}>
      <WorkspacePageHeader
        eyebrow="Workspace"
        title="Projects"
        description={`Active workspace: ${activeScopeLabel}. Create isolated workspaces when you want separate conversations, documents, and memory.`}
        actions={
          <Button
            size="small"
            variant="contained"
            onClick={() => setCreateOpen(true)}
          >
            New Project
          </Button>
        }
      />
      <Dialog
        open={createOpen}
        onClose={() => setCreateOpen(false)}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>Create Project</DialogTitle>
        <DialogContent>
          <Stack spacing={2} sx={{ mt: 1 }}>
            <TextField
              fullWidth
              size="small"
              label="Name"
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
            <TextField
              fullWidth
              size="small"
              label="Description"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
            />
            <Alert severity="info">
              Creating a project opens its workspace immediately. Global remains
              available for unscoped work.
            </Alert>
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setCreateOpen(false)}>Cancel</Button>
          <Button
            variant="contained"
            disabled={createMutation.isPending || !name.trim()}
            onClick={async () => {
              setError(null);
              try {
                const created = asRecord(await createMutation.mutateAsync());
                const createdId = normalizeProjectId(created.id);
                setName("");
                setDescription("");
                setCreateOpen(false);
                if (createdId) {
                  onOpenProjectWorkspace(createdId);
                }
              } catch (e) {
                setError(errMessage(e));
              }
            }}
          >
            Create
          </Button>
        </DialogActions>
      </Dialog>
      <Grid2 container spacing={2}>
        <Grid2 size={{ xs: 12, lg: 7 }}>
          <Box className="list-shell">
            <TableContainer className="table-shell">
              <Table size="small">
                <TableHead>
                  <TableRow>
                    <TableCell>Name</TableCell>
                    <TableCell>Description</TableCell>
                    <TableCell>Conversations</TableCell>
                    <TableCell>Updated</TableCell>
                    <TableCell align="right">Ops</TableCell>
                  </TableRow>
                </TableHead>
                <TableBody>
                  {projects.length === 0 ? (
                    <TableRow>
                      <TableCell colSpan={5}>
                        <Typography
                          variant="body2"
                          sx={{
                            color: "text.secondary",
                          }}
                        >
                          No projects yet. Global workspace stays available
                          until you want a separated project.
                        </Typography>
                      </TableCell>
                    </TableRow>
                  ) : (
                    projects.map((project) => {
                      const id = normalizeProjectId(project.id);
                      const isActiveProject = id === activeProjectId;
                      return (
                        <TableRow key={id} selected={isActiveProject} hover>
                          <TableCell>
                            <Stack
                              direction="row"
                              spacing={0.75}
                              useFlexGap
                              sx={{
                                alignItems: "center",
                                flexWrap: "wrap",
                              }}
                            >
                              <Typography variant="body2">
                                {str(project.name)}
                              </Typography>
                              {isActiveProject ? (
                                <Chip
                                  size="small"
                                  color="primary"
                                  label="Active workspace"
                                />
                              ) : null}
                            </Stack>
                          </TableCell>
                          <TableCell>{str(project.description)}</TableCell>
                          <TableCell>{counts.get(id) || 0}</TableCell>
                          <TableCell
                            title={
                              humanTs(
                                str(
                                  project.updated_at,
                                  str(project.created_at),
                                ),
                              ).tip
                            }
                          >
                            {
                              humanTs(
                                str(
                                  project.updated_at,
                                  str(project.created_at),
                                ),
                              ).label
                            }
                          </TableCell>
                          <TableCell align="right">
                            <RowOpsMenu
                              actions={[
                                {
                                  label: isActiveProject
                                    ? "Open workspace"
                                    : "Set active workspace",
                                  onClick: () => onOpenProjectWorkspace(id),
                                },
                                {
                                  label: "Edit",
                                  onClick: () => {
                                    const pr = asRecord(project);
                                    setSelectedProject(pr);
                                    setEditForm({
                                      name: str(pr.name, ""),
                                      description: str(pr.description, ""),
                                      system_prompt: str(pr.system_prompt, ""),
                                      personality: str(pr.personality, ""),
                                      tools_filter: str(pr.tools_filter, ""),
                                      active: pr.active !== false,
                                    });
                                  },
                                },
                                {
                                  label: "Delete",
                                  tone: "error",
                                  divider: true,
                                  onClick: () => {
                                    setDeleteProject(asRecord(project));
                                    setDeleteConfirm("");
                                  },
                                },
                              ]}
                              ariaLabel="Project options"
                            />
                          </TableCell>
                        </TableRow>
                      );
                    })
                  )}
                </TableBody>
              </Table>
            </TableContainer>
          </Box>
        </Grid2>
        <Grid2 size={{ xs: 12, lg: 5 }}>
          <QueryTable
            title={
              activeProjectId
                ? `Workspace Conversations: ${activeScopeLabel}`
                : "Recent Conversations"
            }
            path={scopedConversationPath}
            arrayKey="conversations"
            columns={["title", "project_id", "channel", "updated_at"]}
            autoRefresh={autoRefresh}
            emptyLabel={
              activeProjectId
                ? "No conversations exist in the active project yet."
                : "No conversations available yet."
            }
            queryKey="projects-conversation-table"
            pageSize={20}
          />
        </Grid2>
      </Grid2>
      {conversationsQ.error || error ? (
        <Alert severity="error">
          {error || errMessage(conversationsQ.error)}
        </Alert>
      ) : null}
      <Dialog
        open={selectedProject != null}
        onClose={() => setSelectedProject(null)}
        maxWidth="md"
        fullWidth
      >
        <DialogTitle>Edit Project</DialogTitle>
        <DialogContent>
          <Stack spacing={1.2}>
            <Grid2 container spacing={1}>
              <Grid2 size={{ xs: 12, md: 6 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Name"
                  value={editForm.name}
                  onChange={(e) =>
                    setEditForm((p) => ({ ...p, name: e.target.value }))
                  }
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 6 }}>
                <FormControlLabel
                  control={
                    <Switch
                      checked={editForm.active}
                      onChange={(e) =>
                        setEditForm((p) => ({ ...p, active: e.target.checked }))
                      }
                    />
                  }
                  label="Active"
                />
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Description"
                  value={editForm.description}
                  onChange={(e) =>
                    setEditForm((p) => ({ ...p, description: e.target.value }))
                  }
                />
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <TextField
                  fullWidth
                  size="small"
                  multiline
                  minRows={4}
                  label="System Prompt (optional)"
                  value={editForm.system_prompt}
                  onChange={(e) =>
                    setEditForm((p) => ({
                      ...p,
                      system_prompt: e.target.value,
                    }))
                  }
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 6 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Personality (optional)"
                  value={editForm.personality}
                  onChange={(e) =>
                    setEditForm((p) => ({ ...p, personality: e.target.value }))
                  }
                  placeholder="e.g. friendly"
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 6 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Tools Filter (optional)"
                  value={editForm.tools_filter}
                  onChange={(e) =>
                    setEditForm((p) => ({ ...p, tools_filter: e.target.value }))
                  }
                  placeholder="Comma-separated allowlist"
                />
              </Grid2>
            </Grid2>

            <Stack
              direction="row"
              spacing={1}
              sx={{
                justifyContent: "flex-end",
              }}
            >
              <Button onClick={() => setSelectedProject(null)}>Cancel</Button>
              <Button
                variant="contained"
                disabled={updateMutation.isPending || !editForm.name.trim()}
                onClick={async () => {
                  const id = str(selectedProject?.id, "");
                  if (!id) return;
                  setError(null);
                  try {
                    await updateMutation.mutateAsync({
                      id,
                      body: {
                        name: editForm.name.trim(),
                        description: editForm.description.trim(),
                        system_prompt:
                          editForm.system_prompt.trim() || undefined,
                        personality: editForm.personality.trim() || undefined,
                        tools_filter: editForm.tools_filter.trim() || undefined,
                        active: editForm.active,
                      },
                    });
                  } catch (e) {
                    setError(errMessage(e));
                  }
                }}
              >
                Save
              </Button>
            </Stack>
          </Stack>
        </DialogContent>
      </Dialog>
      <Dialog
        open={deleteProject != null}
        onClose={() => setDeleteProject(null)}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>Delete Project</DialogTitle>
        <DialogContent>
          <Stack spacing={1}>
            <Alert severity="warning">
              This permanently deletes the project and ALL associated data:
              conversations, messages, documents, document chunks, and durable
              memory items.
            </Alert>
            <Typography variant="body2">
              Type the project name to confirm deletion:{" "}
              <b>{str(deleteProject?.name, "")}</b>
            </Typography>
            <TextField
              fullWidth
              size="small"
              label="Project name"
              value={deleteConfirm}
              onChange={(e) => setDeleteConfirm(e.target.value)}
            />
            <Stack
              direction="row"
              spacing={1}
              sx={{
                justifyContent: "flex-end",
              }}
            >
              <Button onClick={() => setDeleteProject(null)}>Cancel</Button>
              <Button
                color="error"
                variant="contained"
                disabled={
                  deleteMutation.isPending ||
                  !str(deleteProject?.id, "").trim() ||
                  deleteConfirm.trim() !== str(deleteProject?.name, "")
                }
                onClick={async () => {
                  const id = str(deleteProject?.id, "");
                  if (!id) return;
                  setError(null);
                  try {
                    await deleteMutation.mutateAsync(id);
                  } catch (e) {
                    setError(errMessage(e));
                  }
                }}
              >
                Delete Permanently
              </Button>
            </Stack>
          </Stack>
        </DialogContent>
      </Dialog>
    </WorkspacePageShell>
  );
}
