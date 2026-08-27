# andrei-os client

Standalone browser client for the `andrei-os` Axum API. Rust does not build or serve this application.

## Development

Run the API from the repository root:

```sh
cargo run
```

Run the client in another terminal:

```sh
cd client
npm install
npm run dev
```

Open <http://127.0.0.1:5173>. Vite proxies requests under `/api` to the Axum server at `http://127.0.0.1:3000`.

## Production build

```sh
npm run build
```

The static production application is written to `client/dist`. Deploy it with a static host or reverse proxy that forwards `/api` to the Axum application. Do not serve `client/dist` from Axum.
