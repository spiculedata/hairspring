// @vitest-environment happy-dom
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { LADDER_HEADING } from '../confirmationWording';
import { GLOSSARY } from '../glossary/glossaryData';
import { ConfirmationLadder } from './ConfirmationLadder';

afterEach(cleanup);

describe('ConfirmationLadder heading', () => {
  it('keeps the heading name to its own words when the definition is open', () => {
    render(<ConfirmationLadder finding="candidate" environmentPinned={false} />);
    fireEvent.click(screen.getByRole('button', { name: LADDER_HEADING }));
    expect(screen.getByText(GLOSSARY.confirmation.definition)).toBeTruthy();
    const heading = screen.getByRole('heading', { level: 2 });
    expect(heading.textContent).toBe(LADDER_HEADING);
    expect(screen.getByRole('region', { name: LADDER_HEADING })).toBeTruthy();
  });
});
