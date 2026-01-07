// scope_test.js - $scope variable definition and reference test
angular.module('scopeApp', [])

// Basic $scope usage
.controller('BasicController', ['$scope', function($scope) {
    // Definition: $scope.users
    $scope.users = [];

    // Definition: $scope.loadUsers
    $scope.loadUsers = function() {
        // Reference: $scope.users
        return $scope.users;
    };

    // Definition: $scope.selectedUser
    $scope.selectedUser = null;

    // Reference: $scope.loadUsers
    $scope.loadUsers();
}])

// $scope in nested functions
.controller('NestedController', ['$scope', function($scope) {
    // Definition: $scope.count (first occurrence)
    $scope.count = 0;

    function init() {
        // This should NOT create a new definition (already defined)
        $scope.count = 10;

        // Definition: $scope.message (in nested function)
        $scope.message = 'Hello';
    }

    function helper() {
        // Reference: $scope.count
        return $scope.count + 1;
    }

    init();
}])

// $inject pattern with $scope
.controller('InjectController', InjectController);

InjectController.$inject = ['$scope', '$timeout'];

function InjectController($scope, $timeout) {
    // Definition: $scope.status
    $scope.status = 'ready';

    // Definition: $scope.doSomething
    $scope.doSomething = function() {
        // Reference: $scope.status
        $scope.status = 'processing';
    };
}

// Controller without $scope (should not register scope properties)
.controller('NoScopeController', ['$http', function($http) {
    // This should NOT be registered (no $scope DI)
    $scope.invalid = true;
}]);
