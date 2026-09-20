import {check} from "./build.mjs"
import {checkReferenceFixture} from "./check-reference-fixture.mjs"

await check()
await checkReferenceFixture()
console.log("release UI kit generated output is current")
