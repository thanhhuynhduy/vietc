import {render, screen} from '@testing-library/react';
import {describe, expect, it} from 'vitest';
import App from './App';

describe('App', () => {
  it('renders the primary VietC sections', () => {
    render(<App />);

    expect(screen.getByRole('heading', {name: /Trải Nghiệm/i})).toBeInTheDocument();
    expect(screen.getByPlaceholderText(/Gõ VNI tại đây/i)).toBeInTheDocument();
    expect(screen.getByText(/An toàn & Bảo mật/i)).toBeInTheDocument();
  });
});