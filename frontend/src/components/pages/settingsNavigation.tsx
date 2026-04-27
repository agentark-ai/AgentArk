import { Avatar, Box, Button, Stack, Tab, Tabs, Typography } from "@mui/material";
import AgentLogo from "../../assets/logo.svg";
import { preloadSettingsTab } from "./workspacePreload";

export type SettingsNavItem = {
  value: number;
  label: string;
};

export type SettingsNavGroup = {
  id: string;
  label: string;
  items: SettingsNavItem[];
};

export const SETTINGS_NAV_GROUPS: SettingsNavGroup[] = [
  {
    id: "setup",
    label: "Setup",
    items: [
      { value: 0, label: "General" },
      { value: 1, label: "Models" },
      { value: 3, label: "Media" },
      { value: 24, label: "Search" },
    ],
  },
  {
    id: "integrations",
    label: "Integrations",
    items: [
      { value: 20, label: "Messaging Channels" },
      { value: 21, label: "Integrations" },
      { value: 26, label: "Companion Devices" },
      { value: 22, label: "Webhooks & APIs" },
      { value: 23, label: "Plugins" },
    ],
  },
  {
    id: "knowledge",
    label: "Knowledge",
    items: [{ value: 8, label: "MCP Servers" }],
  },
  {
    id: "admin",
    label: "Admin",
    items: [
      { value: 14, label: "Data Lifecycle" },
      { value: 6, label: "Observability" },
      { value: 25, label: "Updates" },
    ],
  },
  {
    id: "security",
    label: "Security",
    items: [
      { value: 4, label: "Security" },
      { value: 5, label: "Advanced" },
    ],
  },
];

export const SETTINGS_NAV_ITEMS = SETTINGS_NAV_GROUPS.flatMap(
  (group) => group.items,
);

export function getSelectedSettingsNav(
  tab: number,
  latestPulseNavCount: number,
): SettingsNavItem {
  return (
    SETTINGS_NAV_ITEMS.find((item) => item.value === tab) ||
    (tab === 9
      ? {
          value: 9,
          label:
            latestPulseNavCount > 0
              ? `ArkPulse (${latestPulseNavCount})`
              : "ArkPulse",
        }
      : tab === 11
        ? {
            value: 11,
            label: "Trace",
          }
        : SETTINGS_NAV_ITEMS[0])
  );
}

export function SettingsNavigation({
  tab,
  onTabChange,
}: {
  tab: number;
  onTabChange: (value: number) => void;
}) {
  const preloadTab = (value: number) => preloadSettingsTab(value);

  return (
    <Box className="settings-sidebar">
      <Box className="settings-brand">
        <Avatar src={AgentLogo} variant="rounded" sx={{ width: 28, height: 28 }} />
        <Stack spacing={0.1}>
          <Typography variant="subtitle2">AgentArk</Typography>
          <Typography
            variant="caption"
            sx={{
              color: "text.secondary",
            }}
          >
            Settings
          </Typography>
        </Stack>
      </Box>
      <Stack
        spacing={0.2}
        className="settings-nav-list"
        sx={{ display: { xs: "none", md: "flex" } }}
      >
        {SETTINGS_NAV_GROUPS.map((group, groupIdx) => (
          <Box key={`settings-nav-group-${group.id}`}>
            <Typography className="settings-nav-group-label">
              {group.label}
            </Typography>
            {group.items.map((item) => (
              <Button
                key={`settings-nav-${item.value}`}
                className={`settings-nav-btn${tab === item.value ? " active" : ""}`}
                variant="text"
                onClick={() => onTabChange(item.value)}
                onMouseEnter={() => preloadTab(item.value)}
                onFocus={() => preloadTab(item.value)}
                onTouchStart={() => preloadTab(item.value)}
              >
                <span>{item.label}</span>
              </Button>
            ))}
            {groupIdx < SETTINGS_NAV_GROUPS.length - 1 ? (
              <div className="settings-nav-divider" />
            ) : null}
          </Box>
        ))}
      </Stack>
      <Tabs
        value={tab}
        onChange={(_, value) => onTabChange(Number(value) || 0)}
        variant="scrollable"
        scrollButtons="auto"
        sx={{ display: { xs: "flex", md: "none" } }}
      >
        {SETTINGS_NAV_ITEMS.map((item) => (
          <Tab
            key={`settings-mobile-${item.value}`}
            value={item.value}
            label={item.label}
            onMouseEnter={() => preloadTab(item.value)}
            onFocus={() => preloadTab(item.value)}
            onTouchStart={() => preloadTab(item.value)}
          />
        ))}
      </Tabs>
    </Box>
  );
}
