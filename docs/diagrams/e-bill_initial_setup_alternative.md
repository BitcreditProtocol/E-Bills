# E-bill Initial Status Quo

```mermaid
%%{
  init: {
    'theme': 'base',
    'themeVariables': {
      'primaryColor': '#FFFFFF',
      'primaryTextColor': '#000000',
      'primaryBorderColor': '#000000',
      'lineColor': '#000000',
      'secondaryColor': '#EEEEEE',
      'secondaryBorderColor': '#000000',
      'tertiaryColor': '#FFFFFF'
    }
  }
}%%

graph TD
    S[E-bill\nInitial Status Quo] ---> A[Alice Ltd. in London\nis short on cash]
    B[Bob Corporation in Boston] -- 1. Owes money to --> A
    A -- 2. Wants to purchase\ngoods from ---> C[Charlie GmbH in Vienna\n has long history with Alice]
    C -- 3. Offers 2% discount\nfor 12.5 BTC of goods--> A
    B -- 4. Willing to pay\nwith--> E[e-bill bill of exchange]
    C -- 5. Willing to be paid\nwith--> E
    subgraph -=-=-
        S
        A
        B
        C
        E
    end
```
