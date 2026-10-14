import { useState } from 'react';
import { setToken } from '../api';

export function TokenPrompt({ onDone }: { onDone: () => void }) {
  const [value, setValue] = useState('');
  return (
    <form
      className="card narrow"
      onSubmit={(e) => {
        e.preventDefault();
        setToken(value.trim() || null);
        onDone();
      }}
    >
      <h2>Sign in</h2>
      <p>This server requires an API token. It is kept for this browser tab only.</p>
      <label>
        API token
        <input type="password" autoComplete="off" value={value} onChange={(e) => setValue(e.target.value)} autoFocus />
      </label>
      <button type="submit">Continue</button>
    </form>
  );
}
