# WASM Setup

## Building

Make sure you have all [prerequisites](./prerequisites.md) installed.

You also need the `wasm-pack` tool from [here](https://drager.github.io/wasm-pack/installer/).

```bash
curl https://drager.github.io/wasm-pack/installer/init.sh -sSf | bash
```

### Development

Within the `bcr-ebill-wasm` crate, run these commands to build the project:

```bash
wasm-pack build --dev --target web
```

### Release Build

```bash
wasm-pack build --target web
```

## Running

After building the project for WASM, you'll find the WASM artifacts in the `.crates/bcr-ebill-wasm/pkg` folder including generated TypeScript bindings.

You can run this by serving it to the web, using any local HTTP-Server. For example, you can use [http-server](https://www.npmjs.com/package/http-server).

There are example `index.html` and `main.js` files, which provide a playground to test the created WASM artifacts.

Within the `bcr-ebill-wasm` crate, you can run:

```bash
http-server -c-1 .
```

This way, you can interact with the app at [http://localhost:8080/](http://localhost:8080/).

The database used by Surreal is IndexedDb in the WASM version, so if you clear your IndexedDb (Dev tools -> Storage), you can reset it.
Also, opening the app in another browser, or a private browser window, also starts with a blank slate.

## API

The API can be used in the following way (you can also check more examples in `main.js`):

### API Example

```javascript
import * as wasm from '../pkg/bcr_ebill_wasm.js';

async function start() {
    let config = {
        bitcoin_network: "testnet",
        nostr_relays: ["wss://relay.wildcat0.clowder-dev.minibill.tech"],
        surreal_db_connection: "indxdb://default",
        job_runner_initial_delay_seconds: 1,
        job_runner_check_interval_seconds: 60,
    };

    await wasm.default();
    await wasm.initialize_api(config);

    let notificationApi = wasm.Api.notification();
    let identityApi = wasm.Api.identity();
    let contactApi = wasm.Api.contact();
    let companyApi = wasm.Api.company();
    let billApi = wasm.Api.bill();
    let generalApi = wasm.Api.general();

    let identity;
    // Identity
    try {
        identity = await identityApi.detail();
        console.log("local identity:", identity);
    } catch(err) {
        console.log("No local identity found - creating..");
        await identityApi.create({
            name: "Johanna Smith",
            email: "jsmith@example.com",
            postal_address: {
                country: "AT",
                city: "Vienna",
                zip: "1020",
                address: "street 1",
            }
        });
        identity = await identityApi.detail();
    }

    await notificationApi.subscribe((evt) => {
        console.log("Received event in JS: ", evt);
    });
}

await start();
```

### TypeScript Bindings

We use [Tsify](https://github.com/madonoharu/tsify) for creating TypeScript bindings from Rust types.
This is useful, as API consumers can just use those types to work with the library.

#### API functions

An exception are API functions, where we have to use `#[wasm_bindgen(unchecked_param_type = "TypeName")]`
and `#[wasm_bindgen(unchecked_return_type = "TypeName")]` to document which types we accept and return.

The reason is, that we need to use serialized/deserialized `wasm_bindgen::JsValue` values to communicate with JS,
so we need to annotate which types are actually behind these generic serialization types.

Example:

```rust
    #[wasm_bindgen(unchecked_return_type = "UploadFileResponse")]
    pub async fn upload(
        &self,
        #[wasm_bindgen(unchecked_param_type = "UploadFile")] payload: JsValue,
    ) -> Result<JsValue> {
        let upload_file: UploadFile = serde_wasm_bindgen::from_value(payload)?;
        let upload_file_handler: &dyn UploadFileHandler = &upload_file as &dyn UploadFileHandler;

        get_ctx()
            .file_upload_service
            .validate_attached_file(upload_file_handler)
            .await?;

        let file_upload_response = get_ctx()
            .file_upload_service
            .upload_file(upload_file_handler)
            .await?;

        let res = serde_wasm_bindgen::to_value(&file_upload_response.into_web())?;
        Ok(res)
    }

```

Which leads to

```typescript
export class Bill {
  ...
  upload(payload: UploadFile): Promise<UploadFileResponse>;
  ...
}

export interface UploadFile {
    data: number[];
    extension: string | undefined;
    name: string;
}

export interface UploadFileResponse {
    file_upload_id: string;
}
```

#### Errors

Generally, most API functions are promise-based and return a `Result<T, JSValue>`, where the error type looks like this:

```rust
#[derive(Tsify, Debug, Clone, Serialize)]
struct JsErrorData {
    error: &'static str,
    message: String,
    code: u16,
}
```

Which leads to

```typescript
export interface JsErrorData {
    error: string;
    message: string;
    code: number;
}
```

On the JS side, it's enough to `await` the API functions and use `try/catch` for error-handling, or any other
promise-based error-handling strategy.

#### Enums

There is another exception for enums, which should be represented as e.g. u8. For those, we just use `#[wasm_bindgen]`.

Example:

```rust
#[wasm_bindgen]
#[repr(u8)]
#[derive(
    Debug, Copy, Clone, serde_repr::Serialize_repr, serde_repr::Deserialize_repr, PartialEq, Eq,
)]
pub enum ContactTypeWeb {
    Person = 0,
    Company = 1,
}
```

Which leads to

```typescript
export enum ContactTypeWeb {
  Person = 0,
  Company = 1,
}
```

If we would use `tsify`, it would automatically convert this to a string-based enum.

Example:

```rust
#[derive(Tsify, Debug, Copy, Clone, Serialize, Deserialize)]
pub enum NotificationTypeWeb {
    General,
    Bill,
}
```

Which leads to

```typescript
export type NotificationTypeWeb = "General" | "Bill";
```

### Using the WASM API

The `bcr-ebill-wasm` API is published to this [npm package](https://www.npmjs.com/package/@bitcredit/bcr-ebill-wasm).

You can simply add it to any JS/TS project:

```json
  "dependencies": {
    "@bitcredit/bcr-ebill-wasm": "^0.3.0"
  }
```

If you use `Vite`, you'll have to configure the server to not optimize the WASM dependency:

```javascript
import { defineConfig } from "vite";

export default defineConfig({
    optimizeDeps: {
        exclude: [
            "@bitcredit/bcr-ebill-wasm"
        ]
    }
});
```

With that, you can just import and use the WASM API in JS/TS:

```javascript
import * as wasm from '@bitcredit/bcr-ebill-wasm';

async function start() {
    let config = {
        bitcoin_network: "testnet",
        nostr_relays: ["wss://relay.wildcat0.clowder-dev.minibill.tech"],
        surreal_db_connection: "indxdb://default",
        job_runner_initial_delay_seconds: 1,
  job_runner_check_interval_seconds: 60,
    };
    await wasm.default();
    await wasm.initialize_api(config);

    let generalApi = wasm.Api.general();
    let currencies = await generalApi.currencies();
    console.log("currencies: ", currencies);
}
await start();
```
