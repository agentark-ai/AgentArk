import {
  Alert,
  Box,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Stack,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
  Typography,
} from "@mui/material";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useRef, useState, type ChangeEvent } from "react";
import { api } from "../../api/client";
import { WorkspacePageHeader, WorkspacePageShell } from "../WorkspacePage";
import { errMessage, pickRecords, str } from "./pageHelpers";
import { formatBytes, humanTs, RowOpsMenu } from "./workspaceUiBits";

const REFRESH_MS = 8000;
const INTERNAL_AGENTARK_DOCUMENT_ID_PREFIX = "agentark_knowledge:";
const INTERNAL_AGENTARK_DOCUMENT_CONTENT_TYPE_PREFIX = "application/x-agentark-";

type DocumentsPageProps = {
  autoRefresh: boolean;
};

export default function DocumentsPage({
  autoRefresh,
}: DocumentsPageProps) {
  const queryClient = useQueryClient();
  const [selectedFile, setSelectedFile] = useState<File | null>(null);
  const [selectedFileName, setSelectedFileName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [uploadDialogOpen, setUploadDialogOpen] = useState(false);
  const [uploadSuccess, setUploadSuccess] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  const docsQ = useQuery({
    queryKey: ["documents-manager"],
    queryFn: () => api.rawGet("/documents?limit=100"),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });

  const uploadFileMutation = useMutation({
    mutationFn: async () => {
      if (!selectedFile) throw new Error("No file selected");
      const formData = new FormData();
      formData.append("file", selectedFile, selectedFile.name);
      return api.rawPostForm("/documents/upload-file", formData);
    },
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["documents-manager"] });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) =>
      api.rawDelete(`/documents/${encodeURIComponent(id)}`),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["documents-manager"] });
    },
  });

  const docs = pickRecords(docsQ.data, "documents").filter((doc) => {
    const id = str(doc.id, "").trim();
    const contentType = str(doc.content_type, "").trim().toLowerCase();
    return (
      !id.startsWith(INTERNAL_AGENTARK_DOCUMENT_ID_PREFIX) &&
      !contentType.startsWith(INTERNAL_AGENTARK_DOCUMENT_CONTENT_TYPE_PREFIX)
    );
  });

  const handleFileSelected = async (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    if (!file) return;
    setError(null);
    setSelectedFile(file);
    setSelectedFileName(file.name);
    event.target.value = "";
  };

  return (
    <WorkspacePageShell spacing={1.5}>
      <input
        ref={fileInputRef}
        type="file"
        hidden
        onChange={handleFileSelected}
      />
      <WorkspacePageHeader
        eyebrow="Data"
        title="Documents"
        description="Showing workspace files."
        actions={
          <Button
            variant="contained"
            size="small"
            onClick={() => {
              setUploadDialogOpen(true);
              setUploadSuccess(null);
              setError(null);
            }}
          >
            Upload Document
          </Button>
        }
      />
      <Box className="list-shell">
        <Dialog
          open={uploadDialogOpen}
          onClose={() => {
            if (!uploadFileMutation.isPending) {
              setUploadDialogOpen(false);
              setSelectedFile(null);
              setSelectedFileName("");
              setError(null);
            }
          }}
          maxWidth="sm"
          fullWidth
        >
          <DialogTitle sx={{ pb: 0.5 }}>Upload Document</DialogTitle>
          <DialogContent>
            <Stack spacing={2} sx={{ mt: 1 }}>
              <Typography
                variant="caption"
                sx={{
                  color: "text.secondary",
                }}
              >
                Supports text, PDF/DOCX extraction, images, archives, and other files.
                Non-text files are saved with searchable metadata.
              </Typography>
              {selectedFile ? (
                <Alert severity="info" sx={{ py: 0.5 }}>
                  Selected: {selectedFileName}
                </Alert>
              ) : null}
              {uploadSuccess ? (
                <Alert severity="success" sx={{ py: 0.5 }}>
                  {uploadSuccess}
                </Alert>
              ) : null}
              {error ? (
                <Alert severity="error" sx={{ py: 0.5 }}>
                  {error}
                </Alert>
              ) : null}
              <Stack direction="row" spacing={1}>
                <Button
                  variant="outlined"
                  onClick={() => fileInputRef.current?.click()}
                  disabled={uploadFileMutation.isPending}
                >
                  Choose File
                </Button>
                <Button
                  variant="contained"
                  disabled={uploadFileMutation.isPending || !selectedFile}
                  onClick={async () => {
                    setError(null);
                    setUploadSuccess(null);
                    try {
                      await uploadFileMutation.mutateAsync();
                      setUploadSuccess(
                        `Uploaded ${selectedFileName} successfully.`,
                      );
                      setSelectedFile(null);
                      setSelectedFileName("");
                    } catch (e) {
                      setError(errMessage(e));
                    }
                  }}
                >
                  {uploadFileMutation.isPending ? "Uploading..." : "Upload"}
                </Button>
                {selectedFile && !uploadFileMutation.isPending ? (
                  <Button
                    variant="text"
                    onClick={() => {
                      setSelectedFile(null);
                      setSelectedFileName("");
                      setError(null);
                      setUploadSuccess(null);
                      if (fileInputRef.current) fileInputRef.current.value = "";
                    }}
                  >
                    Clear
                  </Button>
                ) : null}
              </Stack>
            </Stack>
          </DialogContent>
          <DialogActions>
            <Button
              onClick={() => {
                setUploadDialogOpen(false);
                setSelectedFile(null);
                setSelectedFileName("");
                setError(null);
              }}
              disabled={uploadFileMutation.isPending}
            >
              Close
            </Button>
          </DialogActions>
        </Dialog>

        {false ? <Box className="metadata-box" sx={{ mb: 1.25 }}></Box> : null}

        <TableContainer className="table-shell">
          <Table size="small">
            <TableHead>
              <TableRow>
                <TableCell>Filename</TableCell>
                <TableCell>Type</TableCell>
                <TableCell>Chunks</TableCell>
                <TableCell>Size</TableCell>
                <TableCell>Created</TableCell>
                <TableCell>Ops</TableCell>
              </TableRow>
            </TableHead>
            <TableBody>
              {docs.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={6}>
                    <Typography
                      variant="body2"
                      sx={{
                        color: "text.secondary",
                      }}
                    >
                      No documents yet. Click "Upload Document" to add your first file.
                    </Typography>
                  </TableCell>
                </TableRow>
              ) : (
                docs.map((doc) => {
                  const id = str(doc.id, "");
                  return (
                    <TableRow key={id}>
                      <TableCell>{str(doc.filename)}</TableCell>
                      <TableCell>{str(doc.content_type)}</TableCell>
                      <TableCell>{str(doc.chunk_count)}</TableCell>
                      <TableCell>{formatBytes(doc.file_size)}</TableCell>
                      <TableCell title={humanTs(str(doc.created_at)).tip}>
                        {humanTs(str(doc.created_at)).label}
                      </TableCell>
                      <TableCell align="right">
                        <RowOpsMenu
                          actions={[
                            {
                              label: "Delete",
                              tone: "error",
                              onClick: () => deleteMutation.mutate(id),
                            },
                          ]}
                          ariaLabel="Document options"
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
      {docsQ.error || error ? (
        <Alert severity="error">{error || errMessage(docsQ.error)}</Alert>
      ) : null}
    </WorkspacePageShell>
  );
}
