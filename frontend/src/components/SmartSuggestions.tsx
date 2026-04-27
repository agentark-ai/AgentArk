import {
  Box,
  Button,
  Card,
  CardContent,
  Stack,
  Typography,
} from "@mui/material";
import AutoAwesomeRoundedIcon from "@mui/icons-material/AutoAwesomeRounded";
import type { BriefingResponse, RecommendedAction } from "../types";

type Suggestion = {
  id: string;
  title: string;
  detail: string;
  priority: number;
  action: RecommendedAction;
};

type Props = {
  briefing?: BriefingResponse;
  onExecuteAction: (action: RecommendedAction) => void;
  executing: boolean;
};

function mergeSuggestions(briefing?: BriefingResponse): Suggestion[] {
  const items: Suggestion[] = [];
  const actions: RecommendedAction[] = briefing?.recommended_actions || briefing?.recommended_skills || [];

  for (const action of actions) {
    items.push({
      id: action.id,
      title: action.title,
      detail: action.summary || action.description || "",
      priority: 3,
      action,
    });
  }

  items.sort((a, b) => b.priority - a.priority);
  return items.slice(0, 2);
}

export function SmartSuggestions({
  briefing,
  onExecuteAction,
  executing,
}: Props) {
  const suggestions = mergeSuggestions(briefing);

  if (suggestions.length === 0) return null;

  return (
    <Card className="mission-panel mission-panel--adaptive mission-side-panel mission-side-panel--suggestions">
      <CardContent sx={{ p: 1.2, display: "flex", flexDirection: "column" }}>
        <Stack spacing={1.15} className="mission-panel-content">
          <Stack direction="row" spacing={0.75} sx={{
            alignItems: "center"
          }}>
            <AutoAwesomeRoundedIcon sx={{ fontSize: 18, color: "var(--ui-rgba-244-245-247-820)" }} />
            <Box sx={{ flex: 1 }}>
              <Typography variant="body1" sx={{ fontWeight: 700 }}>
                Recommended Actions
              </Typography>
              <Typography variant="caption" sx={{
                color: "text.secondary"
              }}>
                Available next actions from the current brief and active runtime context.
              </Typography>
            </Box>
          </Stack>

          <Stack spacing={0.85} className="mission-panel-section">
            {suggestions.map((suggestion, index) => (
              <Box
                key={suggestion.id}
                className="action-row"
                sx={{
                  p: "8px 10px",
                  background:
                    "linear-gradient(180deg, var(--ui-rgba-24-24-28-920), var(--ui-rgba-15-15-18-880))",
                }}
              >
                <Stack spacing={0.5}>
                  <Stack
                    direction="row"
                    spacing={0.75}
                    useFlexGap
                    sx={{
                      alignItems: "center",
                      flexWrap: "wrap"
                    }}>
                    <Box
                      sx={{
                        width: 22,
                        height: 22,
                        borderRadius: "50%",
                        border: "1px solid var(--ui-rgba-255-255-255-100)",
                        display: "inline-flex",
                        alignItems: "center",
                        justifyContent: "center",
                        color: "var(--ui-rgba-239-241-244-880)",
                        fontSize: "0.68rem",
                        fontWeight: 700,
                        flexShrink: 0,
                      }}
                    >
                      {index + 1}
                    </Box>
                    <Typography
                      variant="body2"
                      className="mission-title-clamp"
                      sx={{
                        fontWeight: 700
                      }}
                    >
                      {suggestion.title}
                    </Typography>
                    <Typography
                      variant="caption"
                      sx={{
                        color: "var(--ui-rgba-173-177-186-620)",
                        textTransform: "uppercase",
                        letterSpacing: 0,
                      }}
                    >
                      Recommended action
                    </Typography>
                  </Stack>
                  <Typography
                    variant="caption"
                    className="mission-detail-clamp"
                    sx={{
                      color: "text.secondary",
                      lineHeight: 1.45
                    }}>
                    {suggestion.detail}
                  </Typography>

                  <Stack
                    direction="row"
                    spacing={0.6}
                    useFlexGap
                    sx={{
                      mt: 0.35,
                      flexWrap: "wrap"
                    }}>
                    <Button
                      variant="contained"
                      size="small"
                      disabled={executing}
                      onClick={() => onExecuteAction(suggestion.action)}
                      sx={{ textTransform: "none", minWidth: 52 }}
                    >
                      Run
                    </Button>
                  </Stack>
                </Stack>
              </Box>
            ))}
          </Stack>
        </Stack>
      </CardContent>
    </Card>
  );
}
