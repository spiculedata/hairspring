import { GuestText } from './GuestText';

const ERROR_CHARS = 300;

/** An error message as escaped text: it is only ever words from the server or the network, and is treated as untrusted anyway. */
export function ErrorLine({ message }: { message: string }) {
  return (
    <p className="error" role="alert">
      <GuestText text={message} max={ERROR_CHARS} expandable={false} />
    </p>
  );
}
