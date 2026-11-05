/** Show the Action in the job's existing Node workflow. */
export function WorkflowDiff() {
  return (
    <div
      className="diff applied"
      role="img"
      aria-label="Give the job contents read and id-token write permissions for Quod Cloud. Add pango07/quod-action at v2 after setup-node. Keep ubuntu-latest, npm ci, and npm test."
    >
      <div className="ctx">jobs:</div>
      <div className="ctx">  test:</div>
      <div className="ctx">    runs-on: ubuntu-latest</div>
      <div className="ctx">    permissions:</div>
      <div className="ctx">      contents: read</div>
      <div className="ctx">      id-token: write</div>
      <div className="ctx">    steps:</div>
      <div className="ctx">      - uses: actions/checkout@v7</div>
      <div className="ctx">      - uses: actions/setup-node@v7</div>
      <div className="ctx">        with:</div>
      <div className="ctx">          node-version: 24.18.0</div>
      <div className="ad">+     - uses: pango07/quod-action@v2</div>
      <div className="ctx">      - run: npm ci</div>
      <div className="ctx">      - run: npm test</div>
    </div>
  );
}
