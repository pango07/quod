`vitest-suppress-warnings.cjs` is copied byte-for-byte from the published
`vitest@5.0.0` package's `suppress-warnings.cjs`. Its SHA-256 is
`a5012da3d2027007781baecda154d343d8c7ad2327b79bec65b1b60a41f42dd1`.
The dispatch regression deliberately verifies the exact source and the
actual captured Node function, rather than trusting a wrapper's name.

`vite-module-runner.mjs` is the exact `dist/node/module-runner.js` from the
published `vite@8.2.0` package, SHA-256
`9e3f0bc4ea6d01e55bb9a73ee3666c750e139340bb84fa46e35f92085f2535b7`.
Its package license and bundled dependency notices are retained in
`vite-module-runner-LICENSE.md`. The continuation regression executes the
actual loader to compare Promise lifecycle and application ordering with
and without observation; a source-name match alone is insufficient.

Vitest is released under the MIT license:

Copyright (c) 2021-Present VoidZero Inc. and Vitest contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
