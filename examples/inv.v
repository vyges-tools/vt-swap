module top ( a, y );
  input a;
  output y;
  wire n1;
  INV u1 ( .A(a), .Y(n1) );
  INV u2 ( .A(n1), .Y(y) );
endmodule
