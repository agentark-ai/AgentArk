// OrbitSwitcher - left rail above the canvas. Lists every orbit owned by
// the active user, lets the user switch between them, create a new one,
// and open the per-orbit settings dialog.
//
// All decisions here are structural: we never branch on a user's typed
// message text. Routing of user clicks to actions is identifier-driven
// (orbit.id), and "what does this look like" is opaque metadata.

import { useCallback, useState, type FormEvent } from "react";
import {
  Box,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  IconButton,
  ListItemIcon,
  ListItemText,
  Menu,
  MenuItem,
  Stack,
  TextField,
  Tooltip,
  Typography,
} from "@mui/material";
import AddRoundedIcon from "@mui/icons-material/AddRounded";
import CheckRoundedIcon from "@mui/icons-material/CheckRounded";
import ExpandMoreRoundedIcon from "@mui/icons-material/ExpandMoreRounded";
import SettingsRoundedIcon from "@mui/icons-material/SettingsRounded";
import { arkorbitApi, type CreateOrbitPayload } from "./api";
import type { Orbit, OrbitId } from "./types";

type Props = {
  orbits: Orbit[];
  activeOrbitId: OrbitId | null;
  onSelect: (id: OrbitId) => void;
  onCreated: (orbit: Orbit) => void;
  onOpenSettings: (id: OrbitId) => void;
};

export function OrbitSwitcher({
  orbits,
  activeOrbitId,
  onSelect,
  onCreated,
  onOpenSettings,
}: Props) {
  const [createOpen, setCreateOpen] = useState(false);
  const [name, setName] = useState("");
  const [icon, setIcon] = useState("");
  const [color, setColor] = useState("#78f2b0");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [menuAnchor, setMenuAnchor] = useState<HTMLElement | null>(null);
  const activeOrbit = orbits.find((orbit) => orbit.id === activeOrbitId) ?? null;
  const menuOpen = Boolean(menuAnchor);
  const duplicateCreateName = orbits.some(
    (orbit) =>
      orbit.name.trim().toLocaleLowerCase() === name.trim().toLocaleLowerCase(),
  );

  const reset = useCallback(() => {
    setName("");
    setIcon("");
    setColor("#78f2b0");
    setError(null);
  }, []);

  const handleSubmit = useCallback(
    async (event?: FormEvent<HTMLFormElement>) => {
      event?.preventDefault();
      const trimmed = name.trim();
      if (!trimmed) {
        setError("Name is required.");
        return;
      }
      if (
        orbits.some(
          (orbit) => orbit.name.trim().toLocaleLowerCase() === trimmed.toLocaleLowerCase(),
        )
      ) {
        setError("A canvas with this name already exists.");
        return;
      }
      setSubmitting(true);
      setError(null);
      try {
        const payload: CreateOrbitPayload = { name: trimmed };
        if (icon.trim()) payload.icon = icon.trim();
        if (color.trim()) payload.color = color.trim();
        const orbit = await arkorbitApi.createOrbit(payload);
        if (!orbit) {
          setError("Server did not return the new orbit.");
          return;
        }
        onCreated(orbit);
        setCreateOpen(false);
        reset();
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        setSubmitting(false);
      }
    },
    [name, icon, color, onCreated, orbits, reset],
  );

  return (
    <Box className="arkorbit-switcher">
      <Stack direction="row" className="arkorbit-switcher-track" spacing={1}>
        <Button
          className="arkorbit-orbit-menu-button"
          size="small"
          variant="outlined"
          endIcon={<ExpandMoreRoundedIcon fontSize="small" />}
          disabled={orbits.length === 0}
          onClick={(event) => setMenuAnchor(event.currentTarget)}
        >
          <span
            className="arkorbit-orbit-dot"
            style={
              activeOrbit?.color
                ? { borderColor: activeOrbit.color, boxShadow: `0 0 18px ${activeOrbit.color}` }
                : undefined
            }
          >
            {activeOrbit?.icon || ""}
          </span>
          <span className="arkorbit-orbit-menu-name">
            {activeOrbit?.name || "No orbit"}
          </span>
        </Button>
        <Menu
          anchorEl={menuAnchor}
          open={menuOpen}
          onClose={() => setMenuAnchor(null)}
          slotProps={{ paper: { className: "arkorbit-orbit-menu-paper" } }}
        >
          {orbits.map((orbit) => {
            const active = orbit.id === activeOrbitId;
            return (
              <MenuItem
                key={orbit.id}
                selected={active}
                onClick={() => {
                  onSelect(orbit.id);
                  setMenuAnchor(null);
                }}
              >
                <ListItemIcon className="arkorbit-orbit-menu-check">
                  {active ? <CheckRoundedIcon fontSize="small" /> : null}
                </ListItemIcon>
                <ListItemText
                  primary={orbit.name}
                  secondary={orbit.is_default ? "Default orbit" : undefined}
                />
              </MenuItem>
            );
          })}
        </Menu>
        <Tooltip title="Orbit settings">
          <span>
            <IconButton
              size="small"
              className="arkorbit-toolbar-icon"
              disabled={!activeOrbit}
              onClick={() => {
                if (activeOrbit) onOpenSettings(activeOrbit.id);
              }}
              aria-label={activeOrbit ? `Settings for ${activeOrbit.name}` : "Orbit settings"}
            >
              <SettingsRoundedIcon fontSize="small" />
            </IconButton>
          </span>
        </Tooltip>
        <Tooltip title="New canvas">
          <Button
            size="small"
            variant="outlined"
            className="arkorbit-new-button"
            startIcon={<AddRoundedIcon fontSize="small" />}
            onClick={() => {
              reset();
              setCreateOpen(true);
            }}
          >
            New
          </Button>
        </Tooltip>
      </Stack>

      <Dialog
        open={createOpen}
        onClose={() => (submitting ? undefined : setCreateOpen(false))}
        maxWidth="xs"
        fullWidth
      >
        <DialogTitle>New canvas</DialogTitle>
        <Box component="form" onSubmit={handleSubmit}>
          <DialogContent>
            <Stack spacing={2}>
              <TextField
                autoFocus
                label="Name"
                value={name}
                onChange={(e) => {
                  setName(e.target.value);
                  setError(null);
                }}
                size="small"
                fullWidth
                error={duplicateCreateName}
                helperText={duplicateCreateName ? "A canvas with this name already exists." : " "}
              />
              <TextField
                label="Icon (emoji or short glyph, optional)"
                value={icon}
                onChange={(e) => setIcon(e.target.value)}
                size="small"
                fullWidth
                slotProps={{ htmlInput: { maxLength: 8 } }}
              />
              <TextField
                label="Color"
                type="color"
                value={color}
                onChange={(e) => setColor(e.target.value)}
                size="small"
                fullWidth
              />
              {error ? (
                <Typography variant="caption" color="error">
                  {error}
                </Typography>
              ) : null}
            </Stack>
          </DialogContent>
          <DialogActions>
            <Button
              onClick={() => setCreateOpen(false)}
              disabled={submitting}
              size="small"
            >
              Cancel
            </Button>
            <Button
              type="submit"
              variant="contained"
              size="small"
              disabled={
                submitting ||
                !name.trim() ||
                duplicateCreateName
              }
            >
              Create
            </Button>
          </DialogActions>
        </Box>
      </Dialog>
    </Box>
  );
}

export default OrbitSwitcher;
