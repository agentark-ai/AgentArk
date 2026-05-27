FROM mcr.microsoft.com/playwright:v1.58.2-noble

ENV NODE_ENV=production

WORKDIR /app

RUN useradd --create-home --shell /usr/sbin/nologin bridge && chown -R bridge:bridge /app

COPY package.json package-lock.json ./
RUN npm ci --omit=dev && npm cache clean --force

COPY --chown=bridge:bridge index.js ./
COPY --chown=bridge:bridge manual-login.js ./

USER bridge

EXPOSE 3100

HEALTHCHECK --interval=30s --timeout=5s --retries=3 \
  CMD node -e "fetch('http://127.0.0.1:3100/health').then(r => r.ok ? process.exit(0) : process.exit(1)).catch(() => process.exit(1))"

CMD ["node", "index.js"]
