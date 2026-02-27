import { Box, Button, Card, CardContent, Stack, Typography } from "@mui/material";
import { useMemo } from "react";

type Props = {
  onGoChat?: () => void;
};

export function WelcomeHero({ onGoChat }: Props) {
  const greeting = useMemo(() => {
    const h = new Date().getHours();
    if (h < 5) return "Welcome back";
    if (h < 12) return "Good morning";
    if (h < 18) return "Good afternoon";
    return "Good evening";
  }, []);

  return (
    <Card
      sx={{
        borderRadius: 2,
        border: "1px solid rgba(108, 156, 212, 0.22)",
        background:
          "linear-gradient(160deg, rgba(9, 21, 39, 0.96), rgba(9, 21, 39, 0.74))," +
          "radial-gradient(circle at 18% 18%, rgba(47, 212, 255, 0.15), rgba(0,0,0,0) 42%)"
      }}
    >
      <CardContent sx={{ p: { xs: 1.25, md: 1.5 } }}>
        <Stack direction="row" spacing={1.2} alignItems="center">
          <Box
            component="img"
            src="/logo.svg"
            alt="AgentArk"
            sx={{
              width: { xs: 56, md: 64 },
              height: { xs: 56, md: 64 },
              flexShrink: 0,
              filter: "drop-shadow(0 0 10px rgba(47, 212, 255, 0.24))"
            }}
          />
          <Box sx={{ flex: 1, minWidth: 0 }}>
            <Typography
              variant="caption"
              sx={{
                display: "block",
                letterSpacing: "0.12em",
                textTransform: "uppercase",
                color: "rgba(170, 214, 247, 0.92)",
                fontWeight: 700,
                mb: 0.25
              }}
            >
              AgentArk
            </Typography>
            <Typography variant="subtitle1" sx={{ fontWeight: 700, lineHeight: 1.3 }}>
              {greeting}. AgentArk is online.
            </Typography>
            <Typography variant="body2" color="text.secondary" sx={{ mt: 0.15 }}>
              Share the target result and I will execute with minimal back-and-forth.
            </Typography>
            <Typography variant="body2" sx={{ mt: 0.35, color: "rgba(196, 230, 255, 0.96)" }}>
              Try: "Review recent changes and list only critical risks."
            </Typography>
          </Box>
          {onGoChat ? (
            <Button size="small" variant="outlined" onClick={onGoChat} sx={{ alignSelf: "center" }}>
              Open Chat
            </Button>
          ) : null}
        </Stack>
      </CardContent>
    </Card>
  );
}
