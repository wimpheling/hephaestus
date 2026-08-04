import {createHash, randomUUID} from "node:crypto";
import http from "node:http";
import {SignJWT, exportJWK, generateKeyPair} from "jose";

const host = "127.0.0.1";
const port = Number(process.env.HEPHAESTUS_E2E_OIDC_PORT ?? "5556");
const issuer = `http://${host}:${port}`;
const webUrl = process.env.HEPHAESTUS_E2E_WEB_URL ?? "http://127.0.0.1:4000";
const redirectUri = `${webUrl}/auth/oidc/callback`;
const clientId = "hephaestus-web";
const clientSecret = "development-secret";
const gitSecret = new TextEncoder().encode(
  "e2e-signing-secret-with-sufficient-entropy"
);
const {publicKey, privateKey} = await generateKeyPair("RS256");
const publicJwk = await exportJWK(publicKey);
Object.assign(publicJwk, {alg: "RS256", use: "sig", kid: "e2e-browser-key"});

const accounts = {
  reviewer: {
    name: "Ada Reviewer",
    email: "ada@example.invalid"
  },
  outsider: {
    name: "Bea Outsider",
    email: "bea@example.invalid"
  }
};

const requests = new Map();
const codes = new Map();
const accessTokens = new Map();

const json = (response, status, value) => {
  const body = JSON.stringify(value);
  response.writeHead(status, {
    "content-type": "application/json",
    "content-length": Buffer.byteLength(body),
    "cache-control": "no-store"
  });
  response.end(body);
};

const redirect = (response, location) => {
  response.writeHead(302, {location, "cache-control": "no-store"});
  response.end();
};

const readForm = async (request) => {
  let body = "";
  for await (const chunk of request) body += chunk;
  return new URLSearchParams(body);
};

const issueBrowserToken = async (authorization, subject, account) => {
  const now = Math.floor(Date.now() / 1000);
  return new SignJWT({
    name: account.name,
    email: account.email,
    email_verified: true,
    nonce: authorization.nonce
  })
    .setProtectedHeader({alg: "RS256", kid: "e2e-browser-key"})
    .setIssuer(issuer)
    .setSubject(subject)
    .setAudience(clientId)
    .setIssuedAt(now)
    .setExpirationTime(now + 300)
    .sign(privateKey);
};

const server = http.createServer(async (request, response) => {
  const url = new URL(request.url, issuer);

  if (request.method === "GET" && url.pathname === "/.well-known/openid-configuration") {
    return json(response, 200, {
      issuer,
      authorization_endpoint: `${issuer}/authorize`,
      token_endpoint: `${issuer}/token`,
      userinfo_endpoint: `${issuer}/userinfo`,
      jwks_uri: `${issuer}/jwks`,
      response_types_supported: ["code"],
      subject_types_supported: ["public"],
      id_token_signing_alg_values_supported: ["RS256"],
      scopes_supported: ["openid", "profile", "email"],
      token_endpoint_auth_methods_supported: [
        "client_secret_post",
        "client_secret_basic"
      ],
      claims_supported: [
        "sub",
        "iss",
        "aud",
        "exp",
        "iat",
        "nonce",
        "name",
        "email",
        "email_verified"
      ],
      code_challenge_methods_supported: ["S256"]
    });
  }

  if (request.method === "GET" && url.pathname === "/jwks") {
    return json(response, 200, {keys: [publicJwk]});
  }

  if (request.method === "GET" && url.pathname === "/authorize") {
    if (
      url.searchParams.get("client_id") !== clientId ||
      url.searchParams.get("redirect_uri") !== redirectUri ||
      url.searchParams.get("response_type") !== "code"
    ) {
      return json(response, 400, {error: "invalid_request"});
    }

    const requestId = randomUUID();
    requests.set(requestId, Object.fromEntries(url.searchParams));
    const body = `<!doctype html>
      <html><head><meta charset="utf-8"><title>Fixture identity</title>
      <style>
        body{font-family:system-ui;background:#f3f1ea;color:#171714;display:grid;place-items:center;min-height:100vh;margin:0}
        main{width:min(28rem,90vw);padding:2rem;background:#fbfaf6;border:1px solid #d8d4c7;border-radius:.7rem;box-shadow:0 20px 60px #2d271c18}
        b{color:#e55227;letter-spacing:.15em;font-size:.7rem}h1{font-family:Georgia;font-weight:500;font-size:2.2rem}
        input,button{width:100%;box-sizing:border-box;padding:.8rem;margin-top:.7rem;border:1px solid #d8d4c7;border-radius:.4rem}
        button{background:#e55227;color:white;font-weight:800;cursor:pointer}
      </style></head>
      <body><main><b>HEPHAESTUS IDENTITY</b><h1>Sign in to the local forge</h1>
      <form method="post" action="/authorize">
      <input type="hidden" name="request_id" value="${requestId}">
      <label>Account<input name="login" value="reviewer" autocomplete="username"></label>
      <button type="submit">Continue as Ada Reviewer</button>
      </form></main></body></html>`;
    response.writeHead(200, {"content-type": "text/html; charset=utf-8"});
    return response.end(body);
  }

  if (request.method === "POST" && url.pathname === "/authorize") {
    const form = await readForm(request);
    const authorization = requests.get(form.get("request_id"));
    const account = accounts[form.get("login")];
    if (!authorization || !account) {
      return json(response, 400, {error: "access_denied"});
    }
    requests.delete(form.get("request_id"));
    const code = randomUUID();
    codes.set(code, {authorization, login: form.get("login")});
    const callback = new URL(authorization.redirect_uri);
    callback.searchParams.set("code", code);
    callback.searchParams.set("state", authorization.state);
    return redirect(response, callback.toString());
  }

  if (request.method === "POST" && url.pathname === "/token") {
    const form = await readForm(request);
    const pending = codes.get(form.get("code"));
    const authorization = pending?.authorization;
    const account = accounts[pending?.login];
    const basic = request.headers.authorization?.startsWith("Basic ")
      ? Buffer.from(request.headers.authorization.slice(6), "base64")
          .toString()
          .split(":")
      : [];
    const suppliedClient = form.get("client_id") ?? basic[0];
    const suppliedSecret = form.get("client_secret") ?? basic[1];
    if (
      !authorization ||
      suppliedClient !== clientId ||
      suppliedSecret !== clientSecret ||
      form.get("redirect_uri") !== authorization.redirect_uri
    ) {
      return json(response, 400, {error: "invalid_grant"});
    }
    if (authorization.code_challenge) {
      const challenge = createHash("sha256")
        .update(form.get("code_verifier") ?? "")
        .digest("base64url");
      if (challenge !== authorization.code_challenge) {
        return json(response, 400, {error: "invalid_grant"});
      }
    }
    codes.delete(form.get("code"));
    const accessToken = randomUUID();
    accessTokens.set(accessToken, pending.login);
    return json(response, 200, {
      access_token: accessToken,
      token_type: "Bearer",
      expires_in: 300,
      id_token: await issueBrowserToken(authorization, pending.login, account)
    });
  }

  if (request.method === "GET" && url.pathname === "/userinfo") {
    const token = request.headers.authorization?.replace(/^Bearer /, "");
    const account = accounts[accessTokens.get(token)];
    if (!account) {
      return json(response, 401, {error: "invalid_token"});
    }
    return json(response, 200, {
      sub: accessTokens.get(token),
      name: account.name,
      email: account.email,
      email_verified: true
    });
  }

  if (request.method === "GET" && url.pathname === "/test/git-token") {
    const now = Math.floor(Date.now() / 1000);
    const token = await new SignJWT({
      email: "ada@example.invalid",
      email_verified: true
    })
      .setProtectedHeader({alg: "HS256"})
      .setIssuer(issuer)
      .setSubject("reviewer")
      .setAudience("hephaestus-git")
      .setIssuedAt(now)
      .setExpirationTime(now + 600)
      .sign(gitSecret);
    response.writeHead(200, {"content-type": "text/plain"});
    return response.end(token);
  }

  json(response, 404, {error: "not_found"});
});

server.listen(port, host, () => {
  process.stdout.write(`OIDC fixture ready at ${issuer}\n`);
});
