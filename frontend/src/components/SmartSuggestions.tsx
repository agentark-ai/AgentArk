import {
  Box,
  Button,
  Card,
  CardContent,
  Collapse,
  Stack,
  Typography,
} from "@mui/material";
import AutoAwesomeRoundedIcon from "@mui/icons-material/AutoAwesomeRounded";
import { useState } from "react";
import type { BriefingResponse, PredictiveNudge, RecommendedSkill } from "../types";

type Suggestion = {
  id: string;
  title: string;
  detail: string;
  type: "nudge" | "skill";
  priority: number;
  skill?: RecommendedSkill;
  nudge?: PredictiveNudge;
};

type Props = {
  briefing?: BriefingResponse;
  nudges: PredictiveNudge[];
  onExecuteSkill: (skill: RecommendedSkill) => void;
  onSnooze: (id: string) => void;
  onDismiss: (id: string) => void;
  executing: boolean;
  feedbackPending: boolean;
};

function mergeSuggestions(briefing?: BriefingResponse, nudges: PredictiveNudge[] = []): Suggestion[] {
  const items: Suggestion[] = [];

  for (const nudge of nudges) {
    const skill =
      nudge.recommended_skill ||
      ((nudge as unknown as { recommended_action?: RecommendedSkill }).recommended_action);
    items.push({
      id: nudge.id,
      title: nudge.title,
      detail: nudge.detail || "",
      type: "nudge",
      priority: nudge.priority || 0,
      skill: skill || undefined,
      nudge,
    });
  }

  const nudgeIds = new Set(nudges.map((nudge) => nudge.id));
  const skills: RecommendedSkill[] =
    briefing?.recommended_skills ||
    ((briefing as unknown as { recommended_actions?: RecommendedSkill[] })?.recommended_actions || []);
  for (const skill of skills) {
    if (nudgeIds.has(skill.id)) continue;
    items.push({
      id: skill.id,
      title: skill.title,
      detail: skill.summary || skill.description || "",
      type: "skill",
      priority: 3,
      skill,
    });
  }

  items.sort((a, b) => b.priority - a.priority);
  return items.slice(0, 3);
}

export function SmartSuggestions({
  briefing,
  nudges,
  onExecuteSkill,
  onSnooze,
  onDismiss,
  executing,
  feedbackPending,
}: Props) {
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const suggestions = mergeSuggestions(briefing, nudges);

  return (
    <Card className="mission-panel mission-panel--adaptive">
      <CardContent sx={{ p: 1.55, display: "flex", flexDirection: "column" }}>
        <Stack spacing={1.15} className="mission-panel-content">
          <Stack direction="row" alignItems="center" spacing={0.75}>
            <AutoAwesomeRoundedIcon sx={{ fontSize: 18, color: "#2fd4ff" }} />
            <Box sx={{ flex: 1 }}>
              <Typography variant="h6" sx={{ fontWeight: 700 }}>
                Next Best Moves
              </Typography>
              <Typography variant="caption" color="text.secondary">
                High-confidence actions based on current tasks, memory, briefing context, and recent failures.
              </Typography>
            </Box>
          </Stack>

          {suggestions.length === 0 ? (
            <Box className="mission-empty-copy">
              <Typography variant="body2" color="text.secondary">
                No strong next moves right now.
              </Typography>
            </Box>
          ) : (
            <Stack spacing={0.85} className="mission-panel-section">
              {suggestions.map((suggestion, index) => (
                <Box
                  key={suggestion.id}
                  className="action-row"
                  sx={{
                    p: "10px 12px",
                    background: "linear-gradient(180deg, rgba(8, 18, 34, 0.76), rgba(6, 14, 28, 0.72))",
                  }}
                >
                  <Stack spacing={0.5}>
                    <Stack direction="row" spacing={0.75} alignItems="center" useFlexGap flexWrap="wrap">
                      <Box
                        sx={{
                          width: 22,
                          height: 22,
                          borderRadius: "50%",
                          border: "1px solid rgba(94, 184, 243, 0.35)",
                          display: "inline-flex",
                          alignItems: "center",
                          justifyContent: "center",
                          color: "rgba(144, 221, 255, 0.98)",
                          fontSize: "0.72rem",
                          fontWeight: 700,
                          flexShrink: 0,
                        }}
                      >
                        {index + 1}
                      </Box>
                      <Typography variant="body2" fontWeight={700} className="mission-title-clamp">
                        {suggestion.title}
                      </Typography>
                      <Typography
                        variant="caption"
                        sx={{ color: "rgba(141, 192, 231, 0.72)", textTransform: "uppercase", letterSpacing: "0.08em" }}
                      >
                        {suggestion.type === "nudge" ? "Predicted move" : "Recommended skill"}
                      </Typography>
                      {suggestion.nudge?.confidence ? (
                        <Typography variant="caption" sx={{ color: "rgba(132, 216, 255, 0.88)" }}>
                          {Math.round(suggestion.nudge.confidence * 100)}% confidence
                        </Typography>
                      ) : null}
                    </Stack>
                    <Typography variant="caption" color="text.secondary" sx={{ lineHeight: 1.45 }} className="mission-detail-clamp">
                      {suggestion.detail.length > 100 ? `${suggestion.detail.slice(0, 97)}...` : suggestion.detail}
                    </Typography>

                    <Collapse in={expandedId === suggestion.id}>
                      <Box sx={{ mt: 0.5, pl: 0.5, borderLeft: "2px solid rgba(47, 212, 255, 0.3)", py: 0.5 }}>
                        <Typography variant="caption" color="text.secondary" sx={{ display: "block", mb: 0.5 }}>
                          {suggestion.detail}
                        </Typography>
                        {suggestion.nudge?.memory_clues?.map((clue) => (
                          <Typography key={clue.id} variant="caption" color="text.secondary" sx={{ display: "block" }}>
                            {clue.memory_type} memory: {clue.summary}
                          </Typography>
                        ))}
                      </Box>
                    </Collapse>

                    <Stack direction="row" spacing={0.6} mt={0.35} useFlexGap flexWrap="wrap">
                      {suggestion.skill ? (
                        <Button
                          variant="contained"
                          size="small"
                          disabled={executing}
                          onClick={() => onExecuteSkill(suggestion.skill!)}
                          sx={{ textTransform: "none", minWidth: 56 }}
                        >
                          Run
                        </Button>
                      ) : null}
                      <Button
                        variant="outlined"
                        size="small"
                        onClick={() => setExpandedId(expandedId === suggestion.id ? null : suggestion.id)}
                        sx={{ textTransform: "none", minWidth: 62 }}
                      >
                        {expandedId === suggestion.id ? "Less" : "Why?"}
                      </Button>
                      {suggestion.type === "nudge" ? (
                        <>
                          <Button
                            variant="outlined"
                            size="small"
                            disabled={feedbackPending}
                            onClick={() => onSnooze(suggestion.id)}
                            sx={{ textTransform: "none", minWidth: 62 }}
                          >
                            Snooze
                          </Button>
                          <Button
                            variant="text"
                            size="small"
                            color="warning"
                            disabled={feedbackPending}
                            onClick={() => onDismiss(suggestion.id)}
                            sx={{ textTransform: "none", minWidth: 62 }}
                          >
                            Dismiss
                          </Button>
                        </>
                      ) : null}
                    </Stack>
                  </Stack>
                </Box>
              ))}
            </Stack>
          )}
        </Stack>
      </CardContent>
    </Card>
  );
}
