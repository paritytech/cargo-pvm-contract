interface AcroLookup {
    function tokenURI(uint256 tokenId) external view returns (uint64);
    function getURL() external view returns (uint64);
    function fixed3(uint64 a) external view returns (uint64);
}
