import {
  Box,
  Button,
  Card,
  CardContent,
  Stack,
  Typography,
} from "@mui/material";
import AutoAwesomeRoundedIcon from "@mui/icons-material/AutoAwesomeRounded";
import type { BriefingResponse, RecommendedSkill } from "../types";

type Suggestion = {
  id: string;
  title: string;
  detail: string;
  priority: number;
  skill: RecommendedSkill;
};

type Props = {
  briefing?: BriefingResponse;
  onExecuteSkill: (skill: RecommendedSkill) => void;
  executing: boolean;
};

function mergeSuggestions(briefing?: BriefingResponse): Suggestion[] {
  const items: Suggestion[] = [];
  const skills: RecommendedSkill[] =
    briefing?.recommended_skills ||
    ((briefing as unknown as { recommended_actions?: RecommendedSkill[] })?.recommended_actions ||
      []);

  for (const skill of skills) {
    items.push({
      id: skill.id,
      title: skill.title,
      detail: skill.summary || skill.description || "",
      priority: 3,
      skill,
    });
  }

  items.sort((a, b) => b.priority - a.priority);
  return items.slice(0, 2);
}

export function SmartSuggestions({
  briefing,
  onExecuteSkill,
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
            <AutoAwesomeRoundedIcon sx={{ fontSize: 18, color: "rgba(244, 245, 247, 0.82)" }} />
            <Box sx={{ flex: 1 }}>
              <Typography variant="body1" sx={{ fontWeight: 700 }}>
                Recommended Skills
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
                    "linear-gradient(180deg, rgba(24, 24, 28, 0.92), rgba(15, 15, 18, 0.88))",
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
                        border: "1px solid rgba(255, 255, 255, 0.1)",
                        display: "inline-flex",
                        alignItems: "center",
                        justifyContent: "center",
                        color: "rgba(239, 241, 244, 0.88)",
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
                        color: "rgba(173, 177, 186, 0.62)",
                        textTransform: "uppercase",
                        letterSpacing: 0,
                      }}
                    >
                      Recommended skill
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
                      onClick={() => onExecuteSkill(suggestion.skill)}
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
